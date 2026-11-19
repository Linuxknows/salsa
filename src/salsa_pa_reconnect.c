#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <time.h>

#include <pulse/pulseaudio.h>
#include <pulse/ext-device-restore.h>
#include <pulse/ext-device-manager.h>
#include <pulse/ext-stream-restore.h>

/* ============================================================================
 * Salsa Transparent PulseAudio Auto-Reconnect & Device Cache Interposer
 * ============================================================================
 * Loaded via LD_LIBRARY_PATH (pa_reconnect/libpulse.so.0) or LD_PRELOAD so
 * that when lwasapi-audiodg restarts or is briefly interrupted:
 *   1. PA_CONTEXT_FAILED / PA_STREAM_FAILED are suppressed once connected.
 *   2. Cached sink/source/server info is served during the brief restart gap
 *      so clients never receive empty device lists and never blank defaults.
 *   3. A pa_time_event on the app's own mainloop reconnects pa_context,
 *      re-subscribes to events, recreates active playback/record pa_streams,
 *      and emits synthetic subscribe notifications + monotonic stream timing.
 * ============================================================================ */

typedef struct wctx wctx_t;
typedef struct wstream wstream_t;
typedef struct wop wop_t;

/* Real libpulse / libc function pointers */
#ifndef SALSA_PA_PROXY
static void *(*real_dlopen_fn)(const char *, int) = NULL;
static char g_self_path[4096] = {0};
#else
extern const char *const salsa_pa_stub_names[];
extern void *salsa_pa_stub_targets[];
void salsa_pa_proxy_ensure_stubs(void);
#endif
static pthread_once_t g_init_once = PTHREAD_ONCE_INIT;

#define REAL_PA_LIST(X) \
    X(pa_context *, pa_context_new, (pa_mainloop_api *mainloop, const char *name)) \
    X(pa_context *, pa_context_new_with_proplist, (pa_mainloop_api *mainloop, const char *name, const pa_proplist *proplist)) \
    X(pa_context *, pa_context_ref, (pa_context *c)) \
    X(void, pa_context_unref, (pa_context *c)) \
    X(void, pa_context_set_state_callback, (pa_context *c, pa_context_notify_cb_t cb, void *userdata)) \
    X(void, pa_context_set_event_callback, (pa_context *p, pa_context_event_cb_t cb, void *userdata)) \
    X(void, pa_context_set_subscribe_callback, (pa_context *c, pa_context_subscribe_cb_t cb, void *userdata)) \
    X(int, pa_context_errno, (const pa_context *c)) \
    X(int, pa_context_is_pending, (const pa_context *c)) \
    X(pa_context_state_t, pa_context_get_state, (const pa_context *c)) \
    X(int, pa_context_connect, (pa_context *c, const char *server, pa_context_flags_t flags, const pa_spawn_api *api)) \
    X(void, pa_context_disconnect, (pa_context *c)) \
    X(pa_operation *, pa_context_drain, (pa_context *c, pa_context_notify_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_exit_daemon, (pa_context *c, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_default_sink, (pa_context *c, const char *name, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_default_source, (pa_context *c, const char *name, pa_context_success_cb_t cb, void *userdata)) \
    X(int, pa_context_is_local, (const pa_context *c)) \
    X(pa_operation *, pa_context_set_name, (pa_context *c, const char *name, pa_context_success_cb_t cb, void *userdata)) \
    X(const char *, pa_context_get_server, (const pa_context *c)) \
    X(uint32_t, pa_context_get_protocol_version, (const pa_context *c)) \
    X(uint32_t, pa_context_get_server_protocol_version, (const pa_context *c)) \
    X(pa_operation *, pa_context_proplist_update, (pa_context *c, pa_update_mode_t mode, const pa_proplist *p, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_proplist_remove, (pa_context *c, const char *const keys[], pa_context_success_cb_t cb, void *userdata)) \
    X(uint32_t, pa_context_get_index, (const pa_context *s)) \
    X(pa_time_event *, pa_context_rttime_new, (const pa_context *c, pa_usec_t usec, pa_time_event_cb_t cb, void *userdata)) \
    X(void, pa_context_rttime_restart, (const pa_context *c, pa_time_event *e, pa_usec_t usec)) \
    X(size_t, pa_context_get_tile_size, (const pa_context *c, const pa_sample_spec *ss)) \
    X(int, pa_context_load_cookie_from_file, (pa_context *c, const char *cookie_file_path)) \
    X(pa_operation *, pa_context_subscribe, (pa_context *c, pa_subscription_mask_t m, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_server_info, (pa_context *c, pa_server_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_sink_info_by_name, (pa_context *c, const char *name, pa_sink_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_sink_info_by_index, (pa_context *c, uint32_t idx, pa_sink_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_sink_info_list, (pa_context *c, pa_sink_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_sink_volume_by_index, (pa_context *c, uint32_t idx, const pa_cvolume *volume, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_sink_volume_by_name, (pa_context *c, const char *name, const pa_cvolume *volume, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_sink_mute_by_index, (pa_context *c, uint32_t idx, int mute, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_sink_mute_by_name, (pa_context *c, const char *name, int mute, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_suspend_sink_by_name, (pa_context *c, const char *sink_name, int suspend, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_suspend_sink_by_index, (pa_context *c, uint32_t idx, int suspend, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_sink_port_by_index, (pa_context *c, uint32_t idx, const char *port, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_sink_port_by_name, (pa_context *c, const char *name, const char *port, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_source_info_by_name, (pa_context *c, const char *name, pa_source_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_source_info_by_index, (pa_context *c, uint32_t idx, pa_source_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_source_info_list, (pa_context *c, pa_source_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_source_volume_by_index, (pa_context *c, uint32_t idx, const pa_cvolume *volume, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_source_volume_by_name, (pa_context *c, const char *name, const pa_cvolume *volume, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_source_mute_by_index, (pa_context *c, uint32_t idx, int mute, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_source_mute_by_name, (pa_context *c, const char *name, int mute, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_suspend_source_by_name, (pa_context *c, const char *source_name, int suspend, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_suspend_source_by_index, (pa_context *c, uint32_t idx, int suspend, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_source_port_by_index, (pa_context *c, uint32_t idx, const char *port, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_source_port_by_name, (pa_context *c, const char *name, const char *port, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_module_info, (pa_context *c, uint32_t idx, pa_module_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_module_info_list, (pa_context *c, pa_module_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_load_module, (pa_context *c, const char *name, const char *argument, pa_context_index_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_unload_module, (pa_context *c, uint32_t idx, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_send_message_to_object, (pa_context *c, const char *recipient_name, const char *message, const char *message_parameters, pa_context_string_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_client_info, (pa_context *c, uint32_t idx, pa_client_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_client_info_list, (pa_context *c, pa_client_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_kill_client, (pa_context *c, uint32_t idx, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_card_info_by_index, (pa_context *c, uint32_t idx, pa_card_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_card_info_by_name, (pa_context *c, const char *name, pa_card_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_card_info_list, (pa_context *c, pa_card_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_card_profile_by_index, (pa_context *c, uint32_t idx, const char *profile, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_card_profile_by_name, (pa_context *c, const char *name, const char *profile, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_port_latency_offset, (pa_context *c, const char *card_name, const char *port_name, int64_t offset, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_sink_input_info, (pa_context *c, uint32_t idx, pa_sink_input_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_sink_input_info_list, (pa_context *c, pa_sink_input_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_move_sink_input_by_name, (pa_context *c, uint32_t idx, const char *sink_name, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_move_sink_input_by_index, (pa_context *c, uint32_t idx, uint32_t sink_idx, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_sink_input_volume, (pa_context *c, uint32_t idx, const pa_cvolume *volume, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_sink_input_mute, (pa_context *c, uint32_t idx, int mute, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_kill_sink_input, (pa_context *c, uint32_t idx, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_source_output_info, (pa_context *c, uint32_t idx, pa_source_output_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_source_output_info_list, (pa_context *c, pa_source_output_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_move_source_output_by_name, (pa_context *c, uint32_t idx, const char *source_name, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_move_source_output_by_index, (pa_context *c, uint32_t idx, uint32_t source_idx, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_source_output_volume, (pa_context *c, uint32_t idx, const pa_cvolume *volume, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_set_source_output_mute, (pa_context *c, uint32_t idx, int mute, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_kill_source_output, (pa_context *c, uint32_t idx, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_stat, (pa_context *c, pa_stat_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_sample_info_by_name, (pa_context *c, const char *name, pa_sample_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_sample_info_by_index, (pa_context *c, uint32_t idx, pa_sample_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_sample_info_list, (pa_context *c, pa_sample_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_remove_sample, (pa_context *c, const char *name, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_play_sample, (pa_context *c, const char *name, const char *dev, pa_volume_t volume, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_play_sample_with_proplist, (pa_context *c, const char *name, const char *dev, pa_volume_t volume, const pa_proplist *proplist, pa_context_index_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_autoload_info_by_name, (pa_context *c, const char *name, pa_autoload_type_t type, pa_autoload_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_autoload_info_by_index, (pa_context *c, uint32_t idx, pa_autoload_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_get_autoload_info_list, (pa_context *c, pa_autoload_info_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_add_autoload, (pa_context *c, const char *name, pa_autoload_type_t type, const char *module, const char *argument, pa_context_index_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_remove_autoload_by_name, (pa_context *c, const char *name, pa_autoload_type_t type, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_context_remove_autoload_by_index, (pa_context *c, uint32_t idx, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_stream *, pa_stream_new, (pa_context *c, const char *name, const pa_sample_spec *ss, const pa_channel_map *map)) \
    X(pa_stream *, pa_stream_new_with_proplist, (pa_context *c, const char *name, const pa_sample_spec *ss, const pa_channel_map *map, pa_proplist *p)) \
    X(pa_stream *, pa_stream_new_extended, (pa_context *c, const char *name, pa_format_info *const *formats, unsigned int n_formats, pa_proplist *p)) \
    X(void, pa_stream_unref, (pa_stream *s)) \
    X(pa_stream *, pa_stream_ref, (pa_stream *s)) \
    X(pa_stream_state_t, pa_stream_get_state, (const pa_stream *p)) \
    X(pa_context *, pa_stream_get_context, (const pa_stream *p)) \
    X(uint32_t, pa_stream_get_index, (const pa_stream *s)) \
    X(uint32_t, pa_stream_get_device_index, (const pa_stream *s)) \
    X(const char *, pa_stream_get_device_name, (const pa_stream *s)) \
    X(int, pa_stream_is_suspended, (const pa_stream *s)) \
    X(int, pa_stream_is_corked, (const pa_stream *s)) \
    X(int, pa_stream_connect_playback, (pa_stream *s, const char *dev, const pa_buffer_attr *attr, pa_stream_flags_t flags, const pa_cvolume *volume, pa_stream *sync_stream)) \
    X(int, pa_stream_connect_record, (pa_stream *s, const char *dev, const pa_buffer_attr *attr, pa_stream_flags_t flags)) \
    X(int, pa_stream_connect_upload, (pa_stream *s, size_t length)) \
    X(int, pa_stream_finish_upload, (pa_stream *s)) \
    X(int, pa_stream_disconnect, (pa_stream *s)) \
    X(int, pa_stream_begin_write, (pa_stream *p, void **data, size_t *nbytes)) \
    X(int, pa_stream_cancel_write, (pa_stream *p)) \
    X(int, pa_stream_write, (pa_stream *p, const void *data, size_t nbytes, pa_free_cb_t free_cb, int64_t offset, pa_seek_mode_t seek)) \
    X(int, pa_stream_write_ext_free, (pa_stream *p, const void *data, size_t nbytes, pa_free_cb_t free_cb, void *free_cb_data, int64_t offset, pa_seek_mode_t seek)) \
    X(int, pa_stream_peek, (pa_stream *p, const void **data, size_t *nbytes)) \
    X(int, pa_stream_drop, (pa_stream *p)) \
    X(size_t, pa_stream_writable_size, (const pa_stream *p)) \
    X(size_t, pa_stream_readable_size, (const pa_stream *p)) \
    X(pa_operation *, pa_stream_drain, (pa_stream *s, pa_stream_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_stream_update_timing_info, (pa_stream *p, pa_stream_success_cb_t cb, void *userdata)) \
    X(void, pa_stream_set_state_callback, (pa_stream *s, pa_stream_notify_cb_t cb, void *userdata)) \
    X(void, pa_stream_set_write_callback, (pa_stream *p, pa_stream_request_cb_t cb, void *userdata)) \
    X(void, pa_stream_set_read_callback, (pa_stream *p, pa_stream_request_cb_t cb, void *userdata)) \
    X(void, pa_stream_set_overflow_callback, (pa_stream *p, pa_stream_notify_cb_t cb, void *userdata)) \
    X(void, pa_stream_set_underflow_callback, (pa_stream *p, pa_stream_notify_cb_t cb, void *userdata)) \
    X(void, pa_stream_set_started_callback, (pa_stream *p, pa_stream_notify_cb_t cb, void *userdata)) \
    X(void, pa_stream_set_latency_update_callback, (pa_stream *p, pa_stream_notify_cb_t cb, void *userdata)) \
    X(void, pa_stream_set_moved_callback, (pa_stream *p, pa_stream_notify_cb_t cb, void *userdata)) \
    X(void, pa_stream_set_suspended_callback, (pa_stream *p, pa_stream_notify_cb_t cb, void *userdata)) \
    X(void, pa_stream_set_event_callback, (pa_stream *p, pa_stream_event_cb_t cb, void *userdata)) \
    X(void, pa_stream_set_buffer_attr_callback, (pa_stream *p, pa_stream_notify_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_stream_cork, (pa_stream *s, int b, pa_stream_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_stream_flush, (pa_stream *s, pa_stream_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_stream_prebuf, (pa_stream *s, pa_stream_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_stream_trigger, (pa_stream *s, pa_stream_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_stream_set_name, (pa_stream *s, const char *name, pa_stream_success_cb_t cb, void *userdata)) \
    X(int, pa_stream_get_time, (pa_stream *s, pa_usec_t *r_usec)) \
    X(int, pa_stream_get_latency, (pa_stream *s, pa_usec_t *r_usec, int *negative)) \
    X(const pa_timing_info *, pa_stream_get_timing_info, (pa_stream *s)) \
    X(const pa_sample_spec *, pa_stream_get_sample_spec, (pa_stream *s)) \
    X(const pa_channel_map *, pa_stream_get_channel_map, (pa_stream *s)) \
    X(const pa_format_info *, pa_stream_get_format_info, (const pa_stream *s)) \
    X(const pa_buffer_attr *, pa_stream_get_buffer_attr, (pa_stream *s)) \
    X(pa_operation *, pa_stream_set_buffer_attr, (pa_stream *s, const pa_buffer_attr *attr, pa_stream_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_stream_update_sample_rate, (pa_stream *s, uint32_t rate, pa_stream_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_stream_proplist_update, (pa_stream *s, pa_update_mode_t mode, pa_proplist *p, pa_stream_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_stream_proplist_remove, (pa_stream *s, const char *const keys[], pa_stream_success_cb_t cb, void *userdata)) \
    X(int, pa_stream_set_monitor_stream, (pa_stream *s, uint32_t sink_input_idx)) \
    X(uint32_t, pa_stream_get_monitor_stream, (const pa_stream *s)) \
    X(int64_t, pa_stream_get_underflow_index, (const pa_stream *p)) \
    X(pa_operation *, pa_operation_ref, (pa_operation *o)) \
    X(void, pa_operation_unref, (pa_operation *o)) \
    X(void, pa_operation_cancel, (pa_operation *o)) \
    X(pa_operation_state_t, pa_operation_get_state, (const pa_operation *o)) \
    X(void, pa_operation_set_state_callback, (pa_operation *o, pa_operation_notify_cb_t cb, void *userdata)) \
    X(pa_threaded_mainloop *, pa_threaded_mainloop_new, (void)) \
    X(void, pa_threaded_mainloop_free, (pa_threaded_mainloop *m)) \
    X(pa_mainloop_api *, pa_threaded_mainloop_get_api, (pa_threaded_mainloop *m)) \
    X(void, pa_threaded_mainloop_signal, (pa_threaded_mainloop *m, int wait_for_accept)) \
    X(pa_operation *, pa_ext_device_restore_test, (pa_context *c, pa_ext_device_restore_test_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_device_restore_subscribe, (pa_context *c, int enable, pa_context_success_cb_t cb, void *userdata)) \
    X(void, pa_ext_device_restore_set_subscribe_cb, (pa_context *c, pa_ext_device_restore_subscribe_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_device_restore_read_formats_all, (pa_context *c, pa_ext_device_restore_read_device_formats_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_device_restore_read_formats, (pa_context *c, pa_device_type_t type, uint32_t idx, pa_ext_device_restore_read_device_formats_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_device_restore_save_formats, (pa_context *c, pa_device_type_t type, uint32_t idx, uint8_t n_formats, pa_format_info **formats, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_device_manager_test, (pa_context *c, pa_ext_device_manager_test_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_device_manager_read, (pa_context *c, pa_ext_device_manager_read_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_device_manager_set_device_description, (pa_context *c, const char *device, const char *description, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_device_manager_delete, (pa_context *c, const char *const s[], pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_device_manager_enable_role_device_priority_routing, (pa_context *c, int enable, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_device_manager_reorder_devices_for_role, (pa_context *c, const char *role, const char **devices, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_device_manager_subscribe, (pa_context *c, int enable, pa_context_success_cb_t cb, void *userdata)) \
    X(void, pa_ext_device_manager_set_subscribe_cb, (pa_context *c, pa_ext_device_manager_subscribe_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_stream_restore_test, (pa_context *c, pa_ext_stream_restore_test_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_stream_restore_read, (pa_context *c, pa_ext_stream_restore_read_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_stream_restore_write, (pa_context *c, pa_update_mode_t mode, const pa_ext_stream_restore_info data[], unsigned n, int apply_immediately, pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_stream_restore_delete, (pa_context *c, const char *const s[], pa_context_success_cb_t cb, void *userdata)) \
    X(pa_operation *, pa_ext_stream_restore_subscribe, (pa_context *c, int enable, pa_context_success_cb_t cb, void *userdata)) \
    X(void, pa_ext_stream_restore_set_subscribe_cb, (pa_context *c, pa_ext_stream_restore_subscribe_cb_t cb, void *userdata))

#define DECLARE_REAL(ret, name, args) static ret (*real_##name) args = NULL;
REAL_PA_LIST(DECLARE_REAL)
#undef DECLARE_REAL

static void ensure_init(void);

#ifdef SALSA_PA_PROXY
static void *g_real_pulse_h = NULL;

static void *open_real_host_libpulse(void) {
    struct stat self_st;
    bool have_self_st = false;
    Dl_info dli;
    if (dladdr((void *)open_real_host_libpulse, &dli) && dli.dli_fname) {
        if (stat(dli.dli_fname, &self_st) == 0) have_self_st = true;
    }

    typedef void *(*dlopen_fn_t)(const char *, int);
    dlopen_fn_t real_dlopen = (dlopen_fn_t)dlsym(RTLD_DEFAULT, "salsa_real_dlopen_passthrough");
    if (!real_dlopen) real_dlopen = dlopen;

    const char *candidates[] = {
#ifdef SALSA_HOST_LIBPULSE
        SALSA_HOST_LIBPULSE,
#endif
#if defined(__i386__)
        "/usr/lib32/libpulse.so.0",
        "/usr/lib/i386-linux-gnu/libpulse.so.0",
        "/lib/i386-linux-gnu/libpulse.so.0",
#else
        "/usr/lib/libpulse.so.0",
        "/usr/lib64/libpulse.so.0",
        "/usr/lib/x86_64-linux-gnu/libpulse.so.0",
        "/lib/x86_64-linux-gnu/libpulse.so.0",
        "/usr/lib/aarch64-linux-gnu/libpulse.so.0",
        "/lib/aarch64-linux-gnu/libpulse.so.0",
#endif
        NULL
    };

    for (int i = 0; candidates[i]; i++) {
        const char *cand = candidates[i];
        if (!cand[0]) continue;
        struct stat st;
        if (stat(cand, &st) != 0) continue;
        if (have_self_st && st.st_dev == self_st.st_dev && st.st_ino == self_st.st_ino) continue;

        char *rp = realpath(cand, NULL);
        const char *open_path = (rp && rp[0]) ? rp : cand;
        void *h = real_dlopen(open_path, RTLD_LAZY | RTLD_LOCAL);
        free(rp);
        if (!h) continue;

        /* Ensure we actually loaded the real host libpulse, not ourselves */
        void *probe = dlsym(h, "pa_context_new");
        if (probe && probe != (void *)pa_context_new) {
            return h;
        }
        dlclose(h);
    }
    return NULL;
}

void salsa_pa_proxy_ensure_stubs(void) {
    ensure_init();
}

__attribute__((destructor))
static void salsa_pa_proxy_dtor(void) {
    if (g_real_pulse_h) {
        dlclose(g_real_pulse_h);
        g_real_pulse_h = NULL;
    }
}
#endif

static void resolve_real_syms(void) {
#ifdef SALSA_PA_PROXY
    void *real_h = open_real_host_libpulse();
    if (!real_h) {
        fprintf(stderr, "[salsa-pa-proxy] ERROR: could not locate real host libpulse.so.0\n");
        return;
    }
    g_real_pulse_h = real_h;

    #define LOAD_REAL(ret, name, args) real_##name = (ret (*) args)dlsym(real_h, #name);
    REAL_PA_LIST(LOAD_REAL)
    #undef LOAD_REAL

    for (size_t i = 0; salsa_pa_stub_names[i] != NULL; i++) {
        salsa_pa_stub_targets[i] = dlsym(real_h, salsa_pa_stub_names[i]);
    }
#else
    if (!real_dlopen_fn) {
        real_dlopen_fn = (void *(*)(const char *, int))dlvsym(RTLD_NEXT, "dlopen", "GLIBC_2.34");
        if (!real_dlopen_fn) real_dlopen_fn = (void *(*)(const char *, int))dlvsym(RTLD_NEXT, "dlopen", "GLIBC_2.2.5");
        if (!real_dlopen_fn) real_dlopen_fn = (void *(*)(const char *, int))dlsym(RTLD_NEXT, "dlopen");
    }

    Dl_info dli;
    if (dladdr((void *)resolve_real_syms, &dli) && dli.dli_fname) {
        snprintf(g_self_path, sizeof(g_self_path), "%s", dli.dli_fname);
    }

    #define LOAD_REAL(ret, name, args) real_##name = (ret (*) args)dlsym(RTLD_NEXT, #name);
    REAL_PA_LIST(LOAD_REAL)
    #undef LOAD_REAL
#endif
}

static void ensure_init(void) {
    pthread_once(&g_init_once, resolve_real_syms);
}

__attribute__((constructor))
static void salsa_pa_reconnect_ctor(void) {
    ensure_init();
}

#ifndef SALSA_PA_PROXY
void *salsa_real_dlopen_passthrough(const char *filename, int flags) {
    if (!real_dlopen_fn) {
        void *(*fn)(const char *, int) = (void *(*)(const char *, int))dlvsym(RTLD_NEXT, "dlopen", "GLIBC_2.34");
        if (!fn) fn = (void *(*)(const char *, int))dlvsym(RTLD_NEXT, "dlopen", "GLIBC_2.2.5");
        if (!fn) fn = (void *(*)(const char *, int))dlsym(RTLD_NEXT, "dlopen");
        if (fn) real_dlopen_fn = fn;
    }
    return real_dlopen_fn ? real_dlopen_fn(filename, flags) : NULL;
}

void *dlopen(const char *filename, int flags) {
    salsa_real_dlopen_passthrough(NULL, 0);
    ensure_init();
    if (filename && strstr(filename, "libpulse.so") &&
        !strstr(filename, "libpulse-simple") &&
        !strstr(filename, "libpulse-mainloop")) {
        if (g_self_path[0] && real_dlopen_fn) {
            void *self_h = real_dlopen_fn(g_self_path, RTLD_LAZY | RTLD_NOLOAD);
            if (self_h) return self_h;
            return real_dlopen_fn(g_self_path, flags);
        }
    }
    return real_dlopen_fn ? real_dlopen_fn(filename, flags) : NULL;
}
#endif

/* ============================================================================
 * Mainloop API <-> pa_threaded_mainloop Registry
 * ============================================================================ */

#define MAX_MAINLOOPS 64
static pthread_mutex_t g_ml_lock = PTHREAD_MUTEX_INITIALIZER;
static struct {
    pa_mainloop_api *api;
    pa_threaded_mainloop *ml;
} g_ml_map[MAX_MAINLOOPS];

static void register_ml(pa_mainloop_api *api, pa_threaded_mainloop *ml) {
    if (!api || !ml) return;
    pthread_mutex_lock(&g_ml_lock);
    for (int i = 0; i < MAX_MAINLOOPS; i++) {
        if (g_ml_map[i].api == api || g_ml_map[i].ml == ml || g_ml_map[i].api == NULL) {
            g_ml_map[i].api = api;
            g_ml_map[i].ml = ml;
            break;
        }
    }
    pthread_mutex_unlock(&g_ml_lock);
}

static void unregister_ml(pa_threaded_mainloop *ml) {
    if (!ml) return;
    pthread_mutex_lock(&g_ml_lock);
    for (int i = 0; i < MAX_MAINLOOPS; i++) {
        if (g_ml_map[i].ml == ml) {
            g_ml_map[i].api = NULL;
            g_ml_map[i].ml = NULL;
        }
    }
    pthread_mutex_unlock(&g_ml_lock);
}

static pa_threaded_mainloop *lookup_ml(pa_mainloop_api *api) {
    if (!api) return NULL;
    pa_threaded_mainloop *res = NULL;
    pthread_mutex_lock(&g_ml_lock);
    for (int i = 0; i < MAX_MAINLOOPS; i++) {
        if (g_ml_map[i].api == api) {
            res = g_ml_map[i].ml;
            break;
        }
    }
    pthread_mutex_unlock(&g_ml_lock);
    return res;
}

/* ============================================================================
 * Synthetic pa_operation Tracker (Used during brief reconnect window)
 * ============================================================================ */

typedef enum {
    SYNTH_KIND_SUCCESS = 0,
    SYNTH_KIND_STREAM_SUCCESS,
    SYNTH_KIND_SERVER_INFO,
    SYNTH_KIND_SINK_LIST,
    SYNTH_KIND_SINK_BY_NAME,
    SYNTH_KIND_SINK_BY_INDEX,
    SYNTH_KIND_SOURCE_LIST,
    SYNTH_KIND_SOURCE_BY_NAME,
    SYNTH_KIND_SOURCE_BY_INDEX,
    SYNTH_KIND_EMPTY_EOL
} synth_kind_t;

struct wop {
    _Atomic int ref;
    pa_operation_state_t state;
    pa_operation_notify_cb_t state_cb;
    void *state_userdata;
    wctx_t *wctx;
    wstream_t *stream;
    synth_kind_t kind;
    void *user_cb;
    void *user_data;
    char *target_name;
    uint32_t target_idx;
    pa_defer_event *defer_ev;
    struct wop *next;
};

static pthread_mutex_t g_wop_lock = PTHREAD_MUTEX_INITIALIZER;
static wop_t *g_wop_head = NULL;

static bool is_synth_op(const pa_operation *o) {
    if (!o) return false;
    bool found = false;
    pthread_mutex_lock(&g_wop_lock);
    for (wop_t *p = g_wop_head; p; p = p->next) {
        if ((const void *)p == (const void *)o) {
            found = true;
            break;
        }
    }
    pthread_mutex_unlock(&g_wop_lock);
    return found;
}

/* ============================================================================
 * Device & Server Info Cache
 * ============================================================================ */

typedef struct {
    bool has_server;
    pa_server_info server;
    char *user_name;
    char *host_name;
    char *server_version;
    char *server_name;
    char *default_sink_name;
    char *default_source_name;

    pa_sink_info *sinks;
    size_t n_sinks;

    pa_source_info *sources;
    size_t n_sources;
} pa_cache_t;

static pthread_mutex_t g_global_cache_lock = PTHREAD_MUTEX_INITIALIZER;
static pa_cache_t g_global_cache = {0};

static char *xstrdup(const char *s) {
    return s ? strdup(s) : NULL;
}

static void free_sink_entry(pa_sink_info *s) {
    free((void *)s->name);
    free((void *)s->description);
    free((void *)s->monitor_source_name);
    free((void *)s->driver);
    if (s->proplist) pa_proplist_free(s->proplist);
    if (s->ports) {
        for (uint32_t i = 0; i < s->n_ports; i++) {
            if (s->ports[i]) {
                free((void *)s->ports[i]->name);
                free((void *)s->ports[i]->description);
                free((void *)s->ports[i]->availability_group);
                free(s->ports[i]);
            }
        }
        free(s->ports);
    }
    if (s->formats) {
        for (uint8_t i = 0; i < s->n_formats; i++) {
            if (s->formats[i]) pa_format_info_free(s->formats[i]);
        }
        free(s->formats);
    }
}

static void free_source_entry(pa_source_info *s) {
    free((void *)s->name);
    free((void *)s->description);
    free((void *)s->monitor_of_sink_name);
    free((void *)s->driver);
    if (s->proplist) pa_proplist_free(s->proplist);
    if (s->ports) {
        for (uint32_t i = 0; i < s->n_ports; i++) {
            if (s->ports[i]) {
                free((void *)s->ports[i]->name);
                free((void *)s->ports[i]->description);
                free((void *)s->ports[i]->availability_group);
                free(s->ports[i]);
            }
        }
        free(s->ports);
    }
    if (s->formats) {
        for (uint8_t i = 0; i < s->n_formats; i++) {
            if (s->formats[i]) pa_format_info_free(s->formats[i]);
        }
        free(s->formats);
    }
}

static void copy_sink_entry(pa_sink_info *dst, const pa_sink_info *src) {
    memset(dst, 0, sizeof(*dst));
    *dst = *src;
    dst->name = xstrdup(src->name);
    dst->description = xstrdup(src->description);
    dst->monitor_source_name = xstrdup(src->monitor_source_name);
    dst->driver = xstrdup(src->driver);
    dst->proplist = src->proplist ? pa_proplist_copy(src->proplist) : NULL;
    dst->ports = NULL;
    dst->active_port = NULL;
    if (src->n_ports > 0 && src->ports) {
        dst->ports = calloc(src->n_ports + 1, sizeof(pa_sink_port_info *));
        for (uint32_t i = 0; i < src->n_ports; i++) {
            if (src->ports[i]) {
                pa_sink_port_info *p = calloc(1, sizeof(pa_sink_port_info));
                *p = *src->ports[i];
                p->name = xstrdup(src->ports[i]->name);
                p->description = xstrdup(src->ports[i]->description);
                p->availability_group = xstrdup(src->ports[i]->availability_group);
                dst->ports[i] = p;
                if (src->active_port == src->ports[i]) {
                    dst->active_port = p;
                }
            }
        }
    }
    dst->formats = NULL;
    if (src->n_formats > 0 && src->formats) {
        dst->formats = calloc(src->n_formats, sizeof(pa_format_info *));
        for (uint8_t i = 0; i < src->n_formats; i++) {
            if (src->formats[i]) dst->formats[i] = pa_format_info_copy(src->formats[i]);
        }
    }
}

static void copy_source_entry(pa_source_info *dst, const pa_source_info *src) {
    memset(dst, 0, sizeof(*dst));
    *dst = *src;
    dst->name = xstrdup(src->name);
    dst->description = xstrdup(src->description);
    dst->monitor_of_sink_name = xstrdup(src->monitor_of_sink_name);
    dst->driver = xstrdup(src->driver);
    dst->proplist = src->proplist ? pa_proplist_copy(src->proplist) : NULL;
    dst->ports = NULL;
    dst->active_port = NULL;
    if (src->n_ports > 0 && src->ports) {
        dst->ports = calloc(src->n_ports + 1, sizeof(pa_source_port_info *));
        for (uint32_t i = 0; i < src->n_ports; i++) {
            if (src->ports[i]) {
                pa_source_port_info *p = calloc(1, sizeof(pa_source_port_info));
                *p = *src->ports[i];
                p->name = xstrdup(src->ports[i]->name);
                p->description = xstrdup(src->ports[i]->description);
                p->availability_group = xstrdup(src->ports[i]->availability_group);
                dst->ports[i] = p;
                if (src->active_port == src->ports[i]) {
                    dst->active_port = p;
                }
            }
        }
    }
    dst->formats = NULL;
    if (src->n_formats > 0 && src->formats) {
        dst->formats = calloc(src->n_formats, sizeof(pa_format_info *));
        for (uint8_t i = 0; i < src->n_formats; i++) {
            if (src->formats[i]) dst->formats[i] = pa_format_info_copy(src->formats[i]);
        }
    }
}

static void cache_set_server(pa_cache_t *c, const pa_server_info *info) {
    if (!info) return;
    free(c->user_name);
    free(c->host_name);
    free(c->server_version);
    free(c->server_name);
    free(c->default_sink_name);
    free(c->default_source_name);
    c->server = *info;
    c->user_name = xstrdup(info->user_name);
    c->host_name = xstrdup(info->host_name);
    c->server_version = xstrdup(info->server_version);
    c->server_name = xstrdup(info->server_name);
    c->default_sink_name = xstrdup(info->default_sink_name);
    c->default_source_name = xstrdup(info->default_source_name);
    c->server.user_name = c->user_name;
    c->server.host_name = c->host_name;
    c->server.server_version = c->server_version;
    c->server.server_name = c->server_name;
    c->server.default_sink_name = c->default_sink_name;
    c->server.default_source_name = c->default_source_name;
    c->has_server = true;
}

static void cache_replace_sinks(pa_cache_t *c, pa_sink_info *items, size_t count) {
    for (size_t i = 0; i < c->n_sinks; i++) free_sink_entry(&c->sinks[i]);
    free(c->sinks);
    c->sinks = items;
    c->n_sinks = count;
}

static void cache_replace_sources(pa_cache_t *c, pa_source_info *items, size_t count) {
    for (size_t i = 0; i < c->n_sources; i++) free_source_entry(&c->sources[i]);
    free(c->sources);
    c->sources = items;
    c->n_sources = count;
}

static void cache_free(pa_cache_t *c) {
    free(c->user_name);
    free(c->host_name);
    free(c->server_version);
    free(c->server_name);
    free(c->default_sink_name);
    free(c->default_source_name);
    cache_replace_sinks(c, NULL, 0);
    cache_replace_sources(c, NULL, 0);
    memset(c, 0, sizeof(*c));
}

/* ============================================================================
 * Context & Stream Wrapper Structures
 * ============================================================================ */

struct wstream {
    _Atomic int ref;
    wctx_t *wctx;
    pa_stream *real_stream;
    pa_stream_state_t reported_state;
    bool ever_ready;
    bool user_connected;
    bool user_disconnected;
    bool is_record;
    bool reconnecting;
    int reconnect_retries;

    /* Creation parameters */
    char *name;
    pa_sample_spec ss;
    pa_channel_map cmap;
    bool has_cmap;
    pa_proplist *proplist;
    pa_format_info **formats;
    unsigned int n_formats;
    bool created_extended;

    /* Connection parameters */
    char *dev;
    pa_buffer_attr attr;
    bool has_attr;
    pa_stream_flags_t flags;
    pa_cvolume volume;
    bool has_volume;
    wstream_t *sync_stream;
    int corked;
    bool has_monitor_stream;
    uint32_t monitor_stream;

    /* Cached metadata & monotonic clock continuity across reconnects */
    uint32_t last_stream_index;
    uint32_t last_device_index;
    pa_timing_info last_timing;
    bool has_last_timing;
    pa_usec_t time_offset_us;
    pa_usec_t last_reported_time_us;
    void *dummy_write_buf;
    size_t dummy_write_cap;
    bool using_dummy_write;

    /* Callbacks */
    pa_stream_notify_cb_t state_cb; void *state_ud;
    pa_stream_request_cb_t write_cb; void *write_ud;
    pa_stream_request_cb_t read_cb; void *read_ud;
    pa_stream_notify_cb_t overflow_cb; void *overflow_ud;
    pa_stream_notify_cb_t underflow_cb; void *underflow_ud;
    pa_stream_notify_cb_t started_cb; void *started_ud;
    pa_stream_notify_cb_t latency_cb; void *latency_ud;
    pa_stream_notify_cb_t moved_cb; void *moved_ud;
    pa_stream_notify_cb_t suspended_cb; void *suspended_ud;
    pa_stream_event_cb_t event_cb; void *event_ud;
    pa_stream_notify_cb_t buffer_attr_cb; void *buffer_attr_ud;

    struct wstream *next;
    struct wstream *global_next;
};

typedef struct dead_handle {
    void *ptr;
    bool is_ctx;
    struct dead_handle *next;
} dead_handle_t;

typedef struct tee_req tee_req_t;

struct wctx {
    _Atomic int ref;
    pthread_mutex_t lock;
    pa_mainloop_api *api;
    pa_threaded_mainloop *tmainloop;
    pa_context *real_ctx;

    char *app_name;
    pa_proplist *props;
    char *server;
    bool server_set;
    pa_context_flags_t flags;
    pa_spawn_api spawn_api;
    bool has_spawn_api;

    pa_context_state_t reported_state;
    int last_errno;
    bool ever_ready;
    bool reconnecting;
    bool user_connected;
    bool user_disconnected;
    int reconnect_attempts;

    pa_context_notify_cb_t state_cb; void *state_ud;
    pa_context_event_cb_t event_cb; void *event_ud;
    pa_context_subscribe_cb_t sub_cb; void *sub_ud;
    bool has_subscribe;
    pa_subscription_mask_t sub_mask;
    pa_ext_device_restore_subscribe_cb_t ext_dev_restore_sub_cb; void *ext_dev_restore_sub_ud;
    bool ext_dev_restore_sub_enabled;
    pa_ext_device_manager_subscribe_cb_t ext_dev_mgr_sub_cb; void *ext_dev_mgr_sub_ud;
    bool ext_dev_mgr_sub_enabled;
    pa_ext_stream_restore_subscribe_cb_t ext_stream_restore_sub_cb; void *ext_stream_restore_sub_ud;
    bool ext_stream_restore_sub_enabled;

    pa_time_event *reconnect_timer;
    pa_defer_event *refresh_defer;

    pa_cache_t cache;
    wstream_t *streams;
    dead_handle_t *dead_handles;
    struct wctx *next;
};

static pthread_mutex_t g_ctx_lock = PTHREAD_MUTEX_INITIALIZER;
static wctx_t *g_ctx_head = NULL;
static pthread_mutex_t g_stream_lock = PTHREAD_MUTEX_INITIALIZER;
static wstream_t *g_stream_head = NULL;

static bool is_valid_wctx(wctx_t *c) {
    if (!c) return false;
    bool ok = false;
    pthread_mutex_lock(&g_ctx_lock);
    for (wctx_t *p = g_ctx_head; p; p = p->next) {
        if (p == c) { ok = true; break; }
    }
    pthread_mutex_unlock(&g_ctx_lock);
    return ok;
}

static bool is_valid_wstream(wstream_t *s) {
    if (!s) return false;
    bool ok = false;
    pthread_mutex_lock(&g_stream_lock);
    for (wstream_t *ws = g_stream_head; ws; ws = ws->global_next) {
        if (ws == s) { ok = true; break; }
    }
    pthread_mutex_unlock(&g_stream_lock);
    return ok;
}

static void internal_ctx_state_cb(pa_context *rc, void *ud);
static void internal_ctx_event_cb(pa_context *rc, const char *name, pa_proplist *p, void *ud);
static void internal_ctx_sub_cb(pa_context *rc, pa_subscription_event_type_t t, uint32_t idx, void *ud);
static void attach_real_stream(wstream_t *ws);
static void replay_inflight_tees(wctx_t *wctx, pa_context *rc);
static void detach_tees_for_wctx(wctx_t *wctx);
static void reap_tees_for_rc(pa_context *rc);

/* ============================================================================
 * Synthetic Operations & Reconnect Timer
 * ============================================================================ */

static void snapshot_stream_time_locked(wstream_t *ws) {
    if (!ws) return;
    if (ws->real_stream && real_pa_stream_get_time) {
        pa_usec_t raw = 0;
        if (real_pa_stream_get_time(ws->real_stream, &raw) == 0) {
            pa_usec_t total = ws->time_offset_us + raw;
            if (total > ws->last_reported_time_us) {
                ws->last_reported_time_us = total;
            }
        }
    }
    ws->time_offset_us = ws->last_reported_time_us;
}

static void wop_free(wop_t *op) {
    if (!op) return;
    pthread_mutex_lock(&g_wop_lock);
    wop_t **pp = &g_wop_head;
    while (*pp) {
        if (*pp == op) {
            *pp = op->next;
            break;
        }
        pp = &(*pp)->next;
    }
    pthread_mutex_unlock(&g_wop_lock);
    if (op->defer_ev && op->wctx && is_valid_wctx(op->wctx) && op->wctx->api) {
        op->wctx->api->defer_free(op->defer_ev);
        op->defer_ev = NULL;
    }
    if (op->stream) {
        pa_stream_unref((pa_stream *)op->stream);
        op->stream = NULL;
    }
    free(op->target_name);
    free(op);
}

static void deliver_synth_op_cb(pa_mainloop_api *a, pa_defer_event *e, void *userdata) {
    wop_t *op = (wop_t *)userdata;
    if (!is_synth_op((const pa_operation *)op)) return;

    if (op->defer_ev == e && a) {
        a->defer_free(e);
        op->defer_ev = NULL;
    }

    if (op->state == PA_OPERATION_RUNNING) {
        wctx_t *wctx = op->wctx;
        if (is_valid_wctx(wctx)) {
            pa_cache_t *cache = &wctx->cache;
            if (!cache->has_server && cache->n_sinks == 0 && cache->n_sources == 0) {
                pthread_mutex_lock(&g_global_cache_lock);
                if (g_global_cache.has_server || g_global_cache.n_sinks > 0 || g_global_cache.n_sources > 0) {
                    cache = &g_global_cache;
                }
                pthread_mutex_unlock(&g_global_cache_lock);
            }

            switch (op->kind) {
            case SYNTH_KIND_SUCCESS:
                if (op->user_cb) {
                    ((pa_context_success_cb_t)op->user_cb)((pa_context *)wctx, 1, op->user_data);
                }
                break;
            case SYNTH_KIND_STREAM_SUCCESS:
                if (op->user_cb && is_valid_wstream(op->stream) && !op->stream->user_disconnected) {
                    ((pa_stream_success_cb_t)op->user_cb)((pa_stream *)op->stream, 1, op->user_data);
                }
                break;
            case SYNTH_KIND_SERVER_INFO:
                if (op->user_cb) {
                    const pa_server_info *si = cache->has_server ? &cache->server : NULL;
                    ((pa_server_info_cb_t)op->user_cb)((pa_context *)wctx, si, op->user_data);
                }
                break;
            case SYNTH_KIND_SINK_LIST:
                if (op->user_cb) {
                    pa_sink_info_cb_t cb = (pa_sink_info_cb_t)op->user_cb;
                    for (size_t i = 0; i < cache->n_sinks; i++) {
                        cb((pa_context *)wctx, &cache->sinks[i], 0, op->user_data);
                    }
                    cb((pa_context *)wctx, NULL, 1, op->user_data);
                }
                break;
            case SYNTH_KIND_SINK_BY_NAME:
                if (op->user_cb) {
                    pa_sink_info_cb_t cb = (pa_sink_info_cb_t)op->user_cb;
                    const char *t = op->target_name;
                    if (t && (strcmp(t, "@DEFAULT_SINK@") == 0 || strcmp(t, "default") == 0)) {
                        t = cache->default_sink_name;
                    }
                    for (size_t i = 0; i < cache->n_sinks; i++) {
                        if ((t && cache->sinks[i].name && strcmp(cache->sinks[i].name, t) == 0) ||
                            (!t && i == 0)) {
                            cb((pa_context *)wctx, &cache->sinks[i], 0, op->user_data);
                            break;
                        }
                    }
                    cb((pa_context *)wctx, NULL, 1, op->user_data);
                }
                break;
            case SYNTH_KIND_SINK_BY_INDEX:
                if (op->user_cb) {
                    pa_sink_info_cb_t cb = (pa_sink_info_cb_t)op->user_cb;
                    for (size_t i = 0; i < cache->n_sinks; i++) {
                        if (cache->sinks[i].index == op->target_idx) {
                            cb((pa_context *)wctx, &cache->sinks[i], 0, op->user_data);
                            break;
                        }
                    }
                    cb((pa_context *)wctx, NULL, 1, op->user_data);
                }
                break;
            case SYNTH_KIND_SOURCE_LIST:
                if (op->user_cb) {
                    pa_source_info_cb_t cb = (pa_source_info_cb_t)op->user_cb;
                    for (size_t i = 0; i < cache->n_sources; i++) {
                        cb((pa_context *)wctx, &cache->sources[i], 0, op->user_data);
                    }
                    cb((pa_context *)wctx, NULL, 1, op->user_data);
                }
                break;
            case SYNTH_KIND_SOURCE_BY_NAME:
                if (op->user_cb) {
                    pa_source_info_cb_t cb = (pa_source_info_cb_t)op->user_cb;
                    const char *t = op->target_name;
                    if (t && (strcmp(t, "@DEFAULT_SOURCE@") == 0 || strcmp(t, "default") == 0)) {
                        t = cache->default_source_name;
                    }
                    for (size_t i = 0; i < cache->n_sources; i++) {
                        if ((t && cache->sources[i].name && strcmp(cache->sources[i].name, t) == 0) ||
                            (!t && i == 0)) {
                            cb((pa_context *)wctx, &cache->sources[i], 0, op->user_data);
                            break;
                        }
                    }
                    cb((pa_context *)wctx, NULL, 1, op->user_data);
                }
                break;
            case SYNTH_KIND_SOURCE_BY_INDEX:
                if (op->user_cb) {
                    pa_source_info_cb_t cb = (pa_source_info_cb_t)op->user_cb;
                    for (size_t i = 0; i < cache->n_sources; i++) {
                        if (cache->sources[i].index == op->target_idx) {
                            cb((pa_context *)wctx, &cache->sources[i], 0, op->user_data);
                            break;
                        }
                    }
                    cb((pa_context *)wctx, NULL, 1, op->user_data);
                }
                break;
            case SYNTH_KIND_EMPTY_EOL:
                if (op->user_cb) {
                    ((pa_sink_input_info_cb_t)op->user_cb)((pa_context *)wctx, NULL, 1, op->user_data);
                }
                break;
            }
        }
        op->state = PA_OPERATION_DONE;
        if (op->state_cb) op->state_cb((pa_operation *)op, op->state_userdata);
        if (is_valid_wctx(wctx) && wctx->tmainloop && real_pa_threaded_mainloop_signal) {
            real_pa_threaded_mainloop_signal(wctx->tmainloop, 0);
        }
    }

    if (atomic_fetch_sub(&op->ref, 1) == 1) {
        wop_free(op);
    }
}

static pa_operation *spawn_synth_op_ex(wctx_t *wctx, wstream_t *stream, synth_kind_t kind,
                                       void *cb, void *ud, const char *target_name, uint32_t target_idx) {
    if (!wctx || !wctx->api || !wctx->api->defer_new) return NULL;
    wop_t *op = calloc(1, sizeof(wop_t));
    if (!op) return NULL;
    atomic_init(&op->ref, 2); /* 1 for caller, 1 for defer_ev */
    op->state = PA_OPERATION_RUNNING;
    op->wctx = wctx;
    op->stream = stream ? (wstream_t *)pa_stream_ref((pa_stream *)stream) : NULL;
    op->kind = kind;
    op->user_cb = cb;
    op->user_data = ud;
    op->target_name = xstrdup(target_name);
    op->target_idx = target_idx;

    pthread_mutex_lock(&g_wop_lock);
    op->next = g_wop_head;
    g_wop_head = op;
    pthread_mutex_unlock(&g_wop_lock);

    op->defer_ev = wctx->api->defer_new(wctx->api, deliver_synth_op_cb, op);
    if (!op->defer_ev) {
        wop_free(op);
        return NULL;
    }
    return (pa_operation *)op;
}

static pa_operation *spawn_synth_op(wctx_t *wctx, synth_kind_t kind, void *cb, void *ud,
                                    const char *target_name, uint32_t target_idx) {
    return spawn_synth_op_ex(wctx, NULL, kind, cb, ud, target_name, target_idx);
}

static pa_operation *spawn_synth_stream_op(wstream_t *ws, void *cb, void *ud) {
    if (!ws || !ws->wctx) return NULL;
    return spawn_synth_op_ex(ws->wctx, ws, SYNTH_KIND_STREAM_SUCCESS, cb, ud, NULL, 0);
}

static void refresh_defer_cb(pa_mainloop_api *a, pa_defer_event *e, void *userdata) {
    wctx_t *wctx = (wctx_t *)userdata;
    if (!is_valid_wctx(wctx)) return;
    if (wctx->refresh_defer == e && a) {
        a->defer_free(e);
        wctx->refresh_defer = NULL;
    }
    if (wctx->sub_cb && wctx->has_subscribe && !wctx->user_disconnected) {
        if (wctx->sub_mask & PA_SUBSCRIPTION_MASK_SERVER) {
            wctx->sub_cb((pa_context *)wctx,
                         (pa_subscription_event_type_t)(PA_SUBSCRIPTION_EVENT_SERVER | PA_SUBSCRIPTION_EVENT_CHANGE),
                         0, wctx->sub_ud);
        }
        if (wctx->sub_mask & PA_SUBSCRIPTION_MASK_SINK) {
            wctx->sub_cb((pa_context *)wctx,
                         (pa_subscription_event_type_t)(PA_SUBSCRIPTION_EVENT_SINK | PA_SUBSCRIPTION_EVENT_NEW),
                         0, wctx->sub_ud);
        }
        if (wctx->sub_mask & PA_SUBSCRIPTION_MASK_SOURCE) {
            wctx->sub_cb((pa_context *)wctx,
                         (pa_subscription_event_type_t)(PA_SUBSCRIPTION_EVENT_SOURCE | PA_SUBSCRIPTION_EVENT_NEW),
                         0, wctx->sub_ud);
        }
    }
    if (wctx->tmainloop && real_pa_threaded_mainloop_signal) {
        real_pa_threaded_mainloop_signal(wctx->tmainloop, 0);
    }
}

static void schedule_reconnect_timer(wctx_t *wctx, uint64_t delay_us);

static void detach_ctx_callbacks(pa_context *rc) {
    if (!rc) return;
    real_pa_context_set_state_callback(rc, NULL, NULL);
    real_pa_context_set_event_callback(rc, NULL, NULL);
    real_pa_context_set_subscribe_callback(rc, NULL, NULL);
    if (real_pa_ext_device_restore_set_subscribe_cb) real_pa_ext_device_restore_set_subscribe_cb(rc, NULL, NULL);
    if (real_pa_ext_device_manager_set_subscribe_cb) real_pa_ext_device_manager_set_subscribe_cb(rc, NULL, NULL);
    if (real_pa_ext_stream_restore_set_subscribe_cb) real_pa_ext_stream_restore_set_subscribe_cb(rc, NULL, NULL);
}

static void detach_stream_callbacks(pa_stream *rs) {
    if (!rs) return;
    real_pa_stream_set_state_callback(rs, NULL, NULL);
    real_pa_stream_set_write_callback(rs, NULL, NULL);
    real_pa_stream_set_read_callback(rs, NULL, NULL);
    real_pa_stream_set_overflow_callback(rs, NULL, NULL);
    real_pa_stream_set_underflow_callback(rs, NULL, NULL);
    real_pa_stream_set_started_callback(rs, NULL, NULL);
    real_pa_stream_set_latency_update_callback(rs, NULL, NULL);
    real_pa_stream_set_moved_callback(rs, NULL, NULL);
    real_pa_stream_set_suspended_callback(rs, NULL, NULL);
    real_pa_stream_set_event_callback(rs, NULL, NULL);
    real_pa_stream_set_buffer_attr_callback(rs, NULL, NULL);
}

static void queue_dead_handle(wctx_t *wctx, void *ptr, bool is_ctx) {
    if (!wctx || !ptr) return;
    dead_handle_t *dh = malloc(sizeof(dead_handle_t));
    if (!dh) {
        if (is_ctx) real_pa_context_unref((pa_context *)ptr);
        else real_pa_stream_unref((pa_stream *)ptr);
        return;
    }
    dh->ptr = ptr;
    dh->is_ctx = is_ctx;
    dh->next = wctx->dead_handles;
    wctx->dead_handles = dh;
}

static dead_handle_t *detach_dead_handles_locked(wctx_t *wctx) {
    if (!wctx) return NULL;
    dead_handle_t *list = wctx->dead_handles;
    wctx->dead_handles = NULL;
    return list;
}

static void flush_dead_handle_list(dead_handle_t *list) {
    if (!list) return;
    for (dead_handle_t *dh = list; dh; dh = dh->next) {
        if (!dh->is_ctx && dh->ptr) {
            real_pa_stream_unref((pa_stream *)dh->ptr);
            dh->ptr = NULL;
        }
    }
    for (dead_handle_t *dh = list; dh; ) {
        dead_handle_t *nx = dh->next;
        if (dh->is_ctx && dh->ptr) {
            pa_context *old_rc = (pa_context *)dh->ptr;
            real_pa_context_unref(old_rc);
            reap_tees_for_rc(old_rc);
        }
        free(dh);
        dh = nx;
    }
}

static void internal_ext_dev_restore_sub_cb(pa_context *rc, pa_device_type_t type, uint32_t idx, void *ud) {
    wctx_t *wctx = (wctx_t *)ud;
    if (!is_valid_wctx(wctx) || wctx->real_ctx != rc) return;
    pa_context_ref((pa_context *)wctx);
    if (wctx->ext_dev_restore_sub_cb) {
        wctx->ext_dev_restore_sub_cb((pa_context *)wctx, type, idx, wctx->ext_dev_restore_sub_ud);
    }
    pa_context_unref((pa_context *)wctx);
}

static void internal_ext_dev_mgr_sub_cb(pa_context *rc, void *ud) {
    wctx_t *wctx = (wctx_t *)ud;
    if (!is_valid_wctx(wctx) || wctx->real_ctx != rc) return;
    pa_context_ref((pa_context *)wctx);
    if (wctx->ext_dev_mgr_sub_cb) {
        wctx->ext_dev_mgr_sub_cb((pa_context *)wctx, wctx->ext_dev_mgr_sub_ud);
    }
    pa_context_unref((pa_context *)wctx);
}

static void internal_ext_stream_restore_sub_cb(pa_context *rc, void *ud) {
    wctx_t *wctx = (wctx_t *)ud;
    if (!is_valid_wctx(wctx) || wctx->real_ctx != rc) return;
    pa_context_ref((pa_context *)wctx);
    if (wctx->ext_stream_restore_sub_cb) {
        wctx->ext_stream_restore_sub_cb((pa_context *)wctx, wctx->ext_stream_restore_sub_ud);
    }
    pa_context_unref((pa_context *)wctx);
}

static void reconnect_timer_cb(pa_mainloop_api *a, pa_time_event *e, const struct timeval *tv, void *userdata) {
    (void)tv;
    wctx_t *wctx = (wctx_t *)userdata;
    if (!is_valid_wctx(wctx)) return;
    pa_context_ref((pa_context *)wctx);

    pthread_mutex_lock(&wctx->lock);
    if (wctx->reconnect_timer == e && a) {
        a->time_free(e);
        wctx->reconnect_timer = NULL;
    }
    dead_handle_t *dead = detach_dead_handles_locked(wctx);
    pa_context *old_rc = NULL;
    if (!wctx->user_disconnected && wctx->reconnecting && wctx->real_ctx) {
        old_rc = wctx->real_ctx;
        wctx->real_ctx = NULL;
        detach_ctx_callbacks(old_rc);
    }
    pthread_mutex_unlock(&wctx->lock);

    flush_dead_handle_list(dead);
    if (old_rc) {
        real_pa_context_unref(old_rc);
        reap_tees_for_rc(old_rc);
    }

    if (!is_valid_wctx(wctx)) {
        pa_context_unref((pa_context *)wctx);
        return;
    }

    pthread_mutex_lock(&wctx->lock);
    if (wctx->user_disconnected || !wctx->reconnecting) {
        pthread_mutex_unlock(&wctx->lock);
        pa_context_unref((pa_context *)wctx);
        return;
    }

    pa_context *nrc = real_pa_context_new_with_proplist(wctx->api, wctx->app_name, wctx->props);
    if (!nrc) {
        schedule_reconnect_timer(wctx, 200000);
        pthread_mutex_unlock(&wctx->lock);
        pa_context_unref((pa_context *)wctx);
        return;
    }

    wctx->real_ctx = nrc;
    real_pa_context_set_state_callback(nrc, internal_ctx_state_cb, wctx);
    real_pa_context_set_event_callback(nrc, internal_ctx_event_cb, wctx);
    real_pa_context_set_subscribe_callback(nrc, internal_ctx_sub_cb, wctx);
    if (wctx->ext_dev_restore_sub_cb && real_pa_ext_device_restore_set_subscribe_cb) {
        real_pa_ext_device_restore_set_subscribe_cb(nrc, internal_ext_dev_restore_sub_cb, wctx);
    }
    if (wctx->ext_dev_mgr_sub_cb && real_pa_ext_device_manager_set_subscribe_cb) {
        real_pa_ext_device_manager_set_subscribe_cb(nrc, internal_ext_dev_mgr_sub_cb, wctx);
    }
    if (wctx->ext_stream_restore_sub_cb && real_pa_ext_stream_restore_set_subscribe_cb) {
        real_pa_ext_stream_restore_set_subscribe_cb(nrc, internal_ext_stream_restore_sub_cb, wctx);
    }

    pa_context_flags_t cflags = (pa_context_flags_t)(wctx->flags & ~PA_CONTEXT_NOFAIL);
    if (real_pa_context_connect(nrc, wctx->server_set ? wctx->server : NULL,
                                cflags, wctx->has_spawn_api ? &wctx->spawn_api : NULL) < 0) {
        if (wctx->real_ctx == nrc) {
            detach_ctx_callbacks(nrc);
            wctx->real_ctx = NULL;
            schedule_reconnect_timer(wctx, 200000);
            pthread_mutex_unlock(&wctx->lock);
            real_pa_context_unref(nrc);
            reap_tees_for_rc(nrc);
            pa_context_unref((pa_context *)wctx);
            return;
        }
    }
    pthread_mutex_unlock(&wctx->lock);
    pa_context_unref((pa_context *)wctx);
}

static void schedule_reconnect_timer(wctx_t *wctx, uint64_t delay_us) {
    if (!wctx || !wctx->api || !wctx->api->time_new || wctx->user_disconnected) return;
    if (wctx->reconnect_timer) {
        wctx->api->time_free(wctx->reconnect_timer);
        wctx->reconnect_timer = NULL;
    }
    struct timeval tv;
    gettimeofday(&tv, NULL);
    tv.tv_usec += (suseconds_t)(delay_us % 1000000ULL);
    tv.tv_sec  += (time_t)(delay_us / 1000000ULL);
    if (tv.tv_usec >= 1000000) {
        tv.tv_sec += 1;
        tv.tv_usec -= 1000000;
    }
    wctx->reconnect_timer = wctx->api->time_new(wctx->api, &tv, reconnect_timer_cb, wctx);
}

/* ============================================================================
 * Internal Context & Stream Callbacks
 * ============================================================================ */

static void internal_ctx_event_cb(pa_context *rc, const char *name, pa_proplist *p, void *ud) {
    wctx_t *wctx = (wctx_t *)ud;
    if (!is_valid_wctx(wctx) || wctx->real_ctx != rc) return;
    if (wctx->event_cb) wctx->event_cb((pa_context *)wctx, name, p, wctx->event_ud);
}

static void internal_ctx_sub_cb(pa_context *rc, pa_subscription_event_type_t t, uint32_t idx, void *ud) {
    wctx_t *wctx = (wctx_t *)ud;
    if (!is_valid_wctx(wctx) || wctx->real_ctx != rc) return;
    if (wctx->sub_cb) wctx->sub_cb((pa_context *)wctx, t, idx, wctx->sub_ud);
}

static void internal_ctx_state_cb(pa_context *rc, void *ud) {
    wctx_t *wctx = (wctx_t *)ud;
    if (!is_valid_wctx(wctx)) return;

    pthread_mutex_lock(&wctx->lock);
    if (wctx->real_ctx != rc) {
        pthread_mutex_unlock(&wctx->lock);
        return;
    }
    pa_context_ref((pa_context *)wctx);

    pa_context_state_t st = real_pa_context_get_state(rc);
    if (st == PA_CONTEXT_READY) {
        bool was_recovery = wctx->ever_ready && wctx->reconnecting;
        wctx->ever_ready = true;
        wctx->reconnecting = false;
        wctx->reconnect_attempts = 0;
        wctx->reported_state = PA_CONTEXT_READY;
        wctx->last_errno = PA_OK;

        if (wctx->reconnect_timer && wctx->api) {
            wctx->api->time_free(wctx->reconnect_timer);
            wctx->reconnect_timer = NULL;
        }

        if (!was_recovery) {
            pa_context_notify_cb_t cb = wctx->state_cb;
            void *cud = wctx->state_ud;
            pthread_mutex_unlock(&wctx->lock);
            if (cb) cb((pa_context *)wctx, cud);
            pa_context_unref((pa_context *)wctx);
            return;
        }

        /* Recovery after Salsa restart */
        if (wctx->has_subscribe) {
            pa_operation *sop = real_pa_context_subscribe(rc, wctx->sub_mask, NULL, NULL);
            if (sop) real_pa_operation_unref(sop);
        }
        if (wctx->ext_dev_restore_sub_enabled && real_pa_ext_device_restore_subscribe) {
            pa_operation *sop = real_pa_ext_device_restore_subscribe(rc, 1, NULL, NULL);
            if (sop) real_pa_operation_unref(sop);
        }
        if (wctx->ext_dev_mgr_sub_enabled && real_pa_ext_device_manager_subscribe) {
            pa_operation *sop = real_pa_ext_device_manager_subscribe(rc, 1, NULL, NULL);
            if (sop) real_pa_operation_unref(sop);
        }
        if (wctx->ext_stream_restore_sub_enabled && real_pa_ext_stream_restore_subscribe) {
            pa_operation *sop = real_pa_ext_stream_restore_subscribe(rc, 1, NULL, NULL);
            if (sop) real_pa_operation_unref(sop);
        }

        for (wstream_t *ws = wctx->streams; ws; ws = ws->next) {
            if (ws->user_connected && !ws->user_disconnected) {
                ws->reconnect_retries = 0;
                attach_real_stream(ws);
            }
        }

        if (wctx->sub_cb && wctx->api && wctx->api->defer_new) {
            if (wctx->refresh_defer) {
                wctx->api->defer_free(wctx->refresh_defer);
            }
            wctx->refresh_defer = wctx->api->defer_new(wctx->api, refresh_defer_cb, wctx);
        }
        if (wctx->tmainloop && real_pa_threaded_mainloop_signal) {
            real_pa_threaded_mainloop_signal(wctx->tmainloop, 0);
        }
        pthread_mutex_unlock(&wctx->lock);
        pa_context_unref((pa_context *)wctx);
        return;
    }

    if (st == PA_CONTEXT_FAILED && !wctx->user_disconnected) {
        wctx->last_errno = real_pa_context_errno(rc);
        wctx->reconnect_attempts++;

        /* If we never connected on initial startup (e.g. Salsa intentionally disabled),
         * retry briefly (~2s) unless PA_CONTEXT_NOFAIL was requested, then surface error. */
        bool allow_retry = wctx->ever_ready || (wctx->flags & PA_CONTEXT_NOFAIL) || (wctx->reconnect_attempts <= 10);
        if (allow_retry) {
            wctx->reconnecting = true;

            /* Tear down dead real_streams while preserving all wrapper state */
            for (wstream_t *ws = wctx->streams; ws; ws = ws->next) {
                if (ws->real_stream) {
                    snapshot_stream_time_locked(ws);
                    pa_stream *old_s = ws->real_stream;
                    ws->real_stream = NULL;
                    ws->reconnecting = ws->user_connected && !ws->user_disconnected;
                    detach_stream_callbacks(old_s);
                    queue_dead_handle(wctx, old_s, false);
                }
            }

            wctx->real_ctx = NULL;
            detach_ctx_callbacks(rc);
            queue_dead_handle(wctx, rc, true);

            schedule_reconnect_timer(wctx, wctx->reconnect_attempts == 1 ? 50000 : 150000);
            pa_threaded_mainloop *tml = wctx->tmainloop;
            pthread_mutex_unlock(&wctx->lock);
            replay_inflight_tees(wctx, rc);
            if (tml && real_pa_threaded_mainloop_signal) {
                real_pa_threaded_mainloop_signal(tml, 0);
            }
            pa_context_unref((pa_context *)wctx);
            return;
        }
    }

    wctx->reported_state = st;
    if (st == PA_CONTEXT_FAILED || st == PA_CONTEXT_TERMINATED) {
        wctx->last_errno = real_pa_context_errno(rc);
    }
    pa_context_notify_cb_t cb = wctx->state_cb;
    void *cud = wctx->state_ud;
    pthread_mutex_unlock(&wctx->lock);
    if (cb) cb((pa_context *)wctx, cud);
    pa_context_unref((pa_context *)wctx);
}

/* Stream trampolines */
static void internal_stream_state_cb(pa_stream *rs, void *ud) {
    wstream_t *ws = (wstream_t *)ud;
    if (!ws) return;
    wctx_t *wctx = NULL;
    pthread_mutex_lock(&g_stream_lock);
    for (wstream_t *p = g_stream_head; p; p = p->global_next) {
        if (p == ws) {
            atomic_fetch_add(&ws->ref, 1);
            wctx = ws->wctx;
            break;
        }
    }
    pthread_mutex_unlock(&g_stream_lock);
    if (!wctx) return;

    pthread_mutex_lock(&wctx->lock);
    if (ws->real_stream != rs) {
        pthread_mutex_unlock(&wctx->lock);
        pa_stream_unref((pa_stream *)ws);
        return;
    }
    pa_stream_state_t st = real_pa_stream_get_state(rs);
    if (st == PA_STREAM_READY) {
        bool was_reconnecting = ws->reconnecting;
        ws->ever_ready = true;
        ws->reconnecting = false;
        ws->reported_state = PA_STREAM_READY;
        ws->last_stream_index = real_pa_stream_get_index(rs);
        ws->last_device_index = real_pa_stream_get_device_index(rs);
        if (real_pa_stream_get_sample_spec) {
            const pa_sample_spec *nss = real_pa_stream_get_sample_spec(rs);
            if (nss && pa_sample_spec_valid(nss)) ws->ss = *nss;
        }
        if (real_pa_stream_get_channel_map) {
            const pa_channel_map *ncm = real_pa_stream_get_channel_map(rs);
            if (ncm && pa_channel_map_valid(ncm)) {
                ws->cmap = *ncm;
                ws->has_cmap = true;
            }
        }
        if (real_pa_stream_get_buffer_attr) {
            const pa_buffer_attr *nba = real_pa_stream_get_buffer_attr(rs);
            if (nba) {
                ws->attr = *nba;
                ws->has_attr = true;
            }
        }

        /* Reconcile cork state in case app called pa_stream_cork during teardown/creating */
        int cur_corked = real_pa_stream_is_corked ? real_pa_stream_is_corked(rs) : -1;
        if (cur_corked >= 0 && ((cur_corked != 0) != (ws->corked != 0)) && real_pa_stream_cork) {
            pa_operation *cop = real_pa_stream_cork(rs, ws->corked ? 1 : 0, NULL, NULL);
            if (cop) real_pa_operation_unref(cop);
        }
        if (was_reconnecting && real_pa_stream_update_timing_info) {
            pa_operation *top = real_pa_stream_update_timing_info(rs, NULL, NULL);
            if (top) real_pa_operation_unref(top);
        }

        if (ws->state_cb && ws->reconnect_retries == 0) {
            pa_stream_notify_cb_t cb = ws->state_cb;
            void *sud = ws->state_ud;
            pthread_mutex_unlock(&wctx->lock);
            cb((pa_stream *)ws, sud);
            pa_stream_unref((pa_stream *)ws);
            return;
        }
        if (wctx->tmainloop && real_pa_threaded_mainloop_signal) {
            real_pa_threaded_mainloop_signal(wctx->tmainloop, 0);
        }
        pthread_mutex_unlock(&wctx->lock);
        pa_stream_unref((pa_stream *)ws);
        return;
    }
    if (st == PA_STREAM_FAILED && ws->user_connected && !ws->user_disconnected && !ws->reconnecting) {
        if (wctx->reconnecting || (wctx->real_ctx && real_pa_context_get_state(wctx->real_ctx) != PA_CONTEXT_READY)) {
            snapshot_stream_time_locked(ws);
            ws->reconnecting = true;
            ws->real_stream = NULL;
            detach_stream_callbacks(rs);
            queue_dead_handle(wctx, rs, false);
            pthread_mutex_unlock(&wctx->lock);
            pa_stream_unref((pa_stream *)ws);
            return;
        }
    }
    if (st == PA_STREAM_FAILED && ws->reconnecting && ws->user_connected && !ws->user_disconnected) {
        /* Target device may not be ready yet after restart; fall back to default device */
        ws->reconnect_retries++;
        if (ws->reconnect_retries <= 3) {
            snapshot_stream_time_locked(ws);
            ws->real_stream = NULL;
            detach_stream_callbacks(rs);
            queue_dead_handle(wctx, rs, false);
            if (ws->reconnect_retries >= 2 && ws->dev) {
                free(ws->dev);
                ws->dev = NULL;
            }
            attach_real_stream(ws);
            pthread_mutex_unlock(&wctx->lock);
            pa_stream_unref((pa_stream *)ws);
            return;
        }
    }
    if (!ws->reconnecting) {
        ws->reported_state = st;
        pa_stream_notify_cb_t cb = ws->state_cb;
        void *sud = ws->state_ud;
        pthread_mutex_unlock(&wctx->lock);
        if (cb) cb((pa_stream *)ws, sud);
        pa_stream_unref((pa_stream *)ws);
        return;
    }
    pthread_mutex_unlock(&wctx->lock);
    pa_stream_unref((pa_stream *)ws);
}

static void internal_stream_write_cb(pa_stream *rs, size_t nbytes, void *ud) {
    wstream_t *ws = (wstream_t *)ud;
    if (!is_valid_wstream(ws) || ws->real_stream != rs) return;
    if (ws->write_cb) ws->write_cb((pa_stream *)ws, nbytes, ws->write_ud);
}

static void internal_stream_read_cb(pa_stream *rs, size_t nbytes, void *ud) {
    wstream_t *ws = (wstream_t *)ud;
    if (!is_valid_wstream(ws) || ws->real_stream != rs) return;
    if (ws->read_cb) ws->read_cb((pa_stream *)ws, nbytes, ws->read_ud);
}

static void internal_stream_overflow_cb(pa_stream *rs, void *ud) {
    wstream_t *ws = (wstream_t *)ud;
    if (!is_valid_wstream(ws) || ws->real_stream != rs) return;
    if (ws->overflow_cb) ws->overflow_cb((pa_stream *)ws, ws->overflow_ud);
}

static void internal_stream_underflow_cb(pa_stream *rs, void *ud) {
    wstream_t *ws = (wstream_t *)ud;
    if (!is_valid_wstream(ws) || ws->real_stream != rs) return;
    if (ws->underflow_cb) ws->underflow_cb((pa_stream *)ws, ws->underflow_ud);
}

static void internal_stream_started_cb(pa_stream *rs, void *ud) {
    wstream_t *ws = (wstream_t *)ud;
    if (!is_valid_wstream(ws) || ws->real_stream != rs) return;
    if (ws->started_cb) ws->started_cb((pa_stream *)ws, ws->started_ud);
}

static void internal_stream_latency_cb(pa_stream *rs, void *ud) {
    wstream_t *ws = (wstream_t *)ud;
    if (!is_valid_wstream(ws) || ws->real_stream != rs) return;
    if (ws->latency_cb) ws->latency_cb((pa_stream *)ws, ws->latency_ud);
}

static void internal_stream_moved_cb(pa_stream *rs, void *ud) {
    wstream_t *ws = (wstream_t *)ud;
    if (!is_valid_wstream(ws) || ws->real_stream != rs) return;
    ws->last_device_index = real_pa_stream_get_device_index(rs);
    if (ws->moved_cb) ws->moved_cb((pa_stream *)ws, ws->moved_ud);
}

static void internal_stream_suspended_cb(pa_stream *rs, void *ud) {
    wstream_t *ws = (wstream_t *)ud;
    if (!is_valid_wstream(ws) || ws->real_stream != rs) return;
    if (ws->suspended_cb) ws->suspended_cb((pa_stream *)ws, ws->suspended_ud);
}

static void internal_stream_event_cb(pa_stream *rs, const char *name, pa_proplist *pl, void *ud) {
    wstream_t *ws = (wstream_t *)ud;
    if (!is_valid_wstream(ws) || ws->real_stream != rs) return;
    if (ws->event_cb) ws->event_cb((pa_stream *)ws, name, pl, ws->event_ud);
}

static void internal_stream_buffer_attr_cb(pa_stream *rs, void *ud) {
    wstream_t *ws = (wstream_t *)ud;
    if (!is_valid_wstream(ws) || ws->real_stream != rs) return;
    if (ws->buffer_attr_cb) ws->buffer_attr_cb((pa_stream *)ws, ws->buffer_attr_ud);
}

static void attach_real_stream(wstream_t *ws) {
    wctx_t *wctx = ws->wctx;
    if (!wctx || !wctx->real_ctx) return;

    if (ws->real_stream) {
        snapshot_stream_time_locked(ws);
        pa_stream *old = ws->real_stream;
        ws->real_stream = NULL;
        detach_stream_callbacks(old);
        queue_dead_handle(wctx, old, false);
    }

    pa_stream *nrs = NULL;
    if (ws->created_extended) {
        nrs = real_pa_stream_new_extended(wctx->real_ctx, ws->name, ws->formats, ws->n_formats, ws->proplist);
    } else {
        nrs = real_pa_stream_new_with_proplist(wctx->real_ctx, ws->name, &ws->ss,
                                               ws->has_cmap ? &ws->cmap : NULL, ws->proplist);
    }
    if (!nrs) return;

    ws->real_stream = nrs;
    real_pa_stream_set_state_callback(nrs, internal_stream_state_cb, ws);
    if (ws->write_cb)       real_pa_stream_set_write_callback(nrs, internal_stream_write_cb, ws);
    if (ws->read_cb)        real_pa_stream_set_read_callback(nrs, internal_stream_read_cb, ws);
    if (ws->overflow_cb)    real_pa_stream_set_overflow_callback(nrs, internal_stream_overflow_cb, ws);
    if (ws->underflow_cb)   real_pa_stream_set_underflow_callback(nrs, internal_stream_underflow_cb, ws);
    if (ws->started_cb)     real_pa_stream_set_started_callback(nrs, internal_stream_started_cb, ws);
    if (ws->latency_cb)     real_pa_stream_set_latency_update_callback(nrs, internal_stream_latency_cb, ws);
    if (ws->moved_cb)       real_pa_stream_set_moved_callback(nrs, internal_stream_moved_cb, ws);
    if (ws->suspended_cb)   real_pa_stream_set_suspended_callback(nrs, internal_stream_suspended_cb, ws);
    if (ws->event_cb)       real_pa_stream_set_event_callback(nrs, internal_stream_event_cb, ws);
    if (ws->buffer_attr_cb) real_pa_stream_set_buffer_attr_callback(nrs, internal_stream_buffer_attr_cb, ws);
    if (ws->has_monitor_stream) {
        real_pa_stream_set_monitor_stream(nrs, ws->monitor_stream);
    }

    pa_stream_flags_t rflags = (pa_stream_flags_t)(ws->flags & ~PA_STREAM_START_CORKED);
    if (ws->corked) rflags = (pa_stream_flags_t)(rflags | PA_STREAM_START_CORKED);

    int rc = -1;
    if (ws->is_record) {
        rc = real_pa_stream_connect_record(nrs, ws->dev, ws->has_attr ? &ws->attr : NULL, rflags);
    } else {
        pa_stream *rs_sync = (ws->sync_stream && is_valid_wstream(ws->sync_stream)) ? ws->sync_stream->real_stream : NULL;
        rc = real_pa_stream_connect_playback(nrs, ws->dev, ws->has_attr ? &ws->attr : NULL, rflags,
                                             ws->has_volume ? &ws->volume : NULL, rs_sync);
    }
    if (rc < 0 && ws->dev) {
        free(ws->dev);
        ws->dev = NULL;
        if (ws->is_record) {
            rc = real_pa_stream_connect_record(nrs, NULL, ws->has_attr ? &ws->attr : NULL, rflags);
        } else {
            rc = real_pa_stream_connect_playback(nrs, NULL, ws->has_attr ? &ws->attr : NULL, rflags,
                                                 ws->has_volume ? &ws->volume : NULL, NULL);
        }
    }
}

/* ============================================================================
 * Mainloop & Operation Wrappers
 * ============================================================================ */

pa_threaded_mainloop *pa_threaded_mainloop_new(void) {
    ensure_init();
    pa_threaded_mainloop *m = real_pa_threaded_mainloop_new();
    if (m && real_pa_threaded_mainloop_get_api) {
        register_ml(real_pa_threaded_mainloop_get_api(m), m);
    }
    return m;
}

void pa_threaded_mainloop_free(pa_threaded_mainloop *m) {
    ensure_init();
    unregister_ml(m);
    real_pa_threaded_mainloop_free(m);
}

pa_mainloop_api *pa_threaded_mainloop_get_api(pa_threaded_mainloop *m) {
    ensure_init();
    pa_mainloop_api *api = real_pa_threaded_mainloop_get_api(m);
    register_ml(api, m);
    return api;
}

pa_operation *pa_operation_ref(pa_operation *o) {
    ensure_init();
    if (is_synth_op(o)) {
        atomic_fetch_add(&((wop_t *)o)->ref, 1);
        return o;
    }
    return real_pa_operation_ref(o);
}

void pa_operation_unref(pa_operation *o) {
    ensure_init();
    if (!o) return;
    if (is_synth_op(o)) {
        wop_t *op = (wop_t *)o;
        if (atomic_fetch_sub(&op->ref, 1) == 1) wop_free(op);
        return;
    }
    real_pa_operation_unref(o);
}

void pa_operation_cancel(pa_operation *o) {
    ensure_init();
    if (!o) return;
    if (is_synth_op(o)) {
        wop_t *op = (wop_t *)o;
        op->state = PA_OPERATION_CANCELLED;
        if (op->state_cb) op->state_cb(o, op->state_userdata);
        return;
    }
    real_pa_operation_cancel(o);
}

pa_operation_state_t pa_operation_get_state(const pa_operation *o) {
    ensure_init();
    if (!o) return PA_OPERATION_CANCELLED;
    if (is_synth_op(o)) {
        return ((const wop_t *)o)->state;
    }
    return real_pa_operation_get_state(o);
}

void pa_operation_set_state_callback(pa_operation *o, pa_operation_notify_cb_t cb, void *userdata) {
    ensure_init();
    if (!o) return;
    if (is_synth_op(o)) {
        wop_t *op = (wop_t *)o;
        op->state_cb = cb;
        op->state_userdata = userdata;
        return;
    }
    real_pa_operation_set_state_callback(o, cb, userdata);
}

/* ============================================================================
 * Context Creation / Lifecycle / Connection
 * ============================================================================ */

pa_context *pa_context_new_with_proplist(pa_mainloop_api *mainloop, const char *name, const pa_proplist *proplist) {
    ensure_init();
    pa_context *rc = real_pa_context_new_with_proplist(mainloop, name, proplist);
    if (!rc) return NULL;

    wctx_t *wctx = calloc(1, sizeof(wctx_t));
    if (!wctx) {
        real_pa_context_unref(rc);
        return NULL;
    }
    atomic_init(&wctx->ref, 1);
    pthread_mutexattr_t mattr;
    pthread_mutexattr_init(&mattr);
    pthread_mutexattr_settype(&mattr, PTHREAD_MUTEX_RECURSIVE);
    pthread_mutex_init(&wctx->lock, &mattr);
    pthread_mutexattr_destroy(&mattr);

    wctx->api = mainloop;
    wctx->tmainloop = lookup_ml(mainloop);
    wctx->real_ctx = rc;
    wctx->app_name = xstrdup(name);
    wctx->props = proplist ? pa_proplist_copy(proplist) : NULL;
    wctx->reported_state = PA_CONTEXT_UNCONNECTED;

    real_pa_context_set_state_callback(rc, internal_ctx_state_cb, wctx);
    real_pa_context_set_event_callback(rc, internal_ctx_event_cb, wctx);
    real_pa_context_set_subscribe_callback(rc, internal_ctx_sub_cb, wctx);

    pthread_mutex_lock(&g_ctx_lock);
    wctx->next = g_ctx_head;
    g_ctx_head = wctx;
    pthread_mutex_unlock(&g_ctx_lock);

    return (pa_context *)wctx;
}

pa_context *pa_context_new(pa_mainloop_api *mainloop, const char *name) {
    return pa_context_new_with_proplist(mainloop, name, NULL);
}

pa_context *pa_context_ref(pa_context *c) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (is_valid_wctx(wctx)) {
        atomic_fetch_add(&wctx->ref, 1);
        return c;
    }
    return real_pa_context_ref(c);
}

void pa_context_unref(pa_context *c) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) {
        if (c) real_pa_context_unref(c);
        return;
    }
    if (atomic_fetch_sub(&wctx->ref, 1) > 1) return;

    pthread_mutex_lock(&g_ctx_lock);
    wctx_t **pp = &g_ctx_head;
    while (*pp) {
        if (*pp == wctx) {
            *pp = wctx->next;
            break;
        }
        pp = &(*pp)->next;
    }
    pthread_mutex_unlock(&g_ctx_lock);

    if (wctx->reconnect_timer && wctx->api) {
        wctx->api->time_free(wctx->reconnect_timer);
        wctx->reconnect_timer = NULL;
    }
    if (wctx->refresh_defer && wctx->api) {
        wctx->api->defer_free(wctx->refresh_defer);
        wctx->refresh_defer = NULL;
    }
    detach_tees_for_wctx(wctx);
    pa_context *orig_rc = wctx->real_ctx;
    wctx->real_ctx = NULL;
    dead_handle_t *dead = detach_dead_handles_locked(wctx);
    if (orig_rc) {
        detach_ctx_callbacks(orig_rc);
        real_pa_context_unref(orig_rc);
        reap_tees_for_rc(orig_rc);
    }
    flush_dead_handle_list(dead);
    free(wctx->app_name);
    if (wctx->props) pa_proplist_free(wctx->props);
    free(wctx->server);
    cache_free(&wctx->cache);
    pthread_mutex_destroy(&wctx->lock);
    free(wctx);
}

void pa_context_set_state_callback(pa_context *c, pa_context_notify_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return;
    pthread_mutex_lock(&wctx->lock);
    wctx->state_cb = cb;
    wctx->state_ud = userdata;
    pthread_mutex_unlock(&wctx->lock);
}

void pa_context_set_event_callback(pa_context *c, pa_context_event_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return;
    pthread_mutex_lock(&wctx->lock);
    wctx->event_cb = cb;
    wctx->event_ud = userdata;
    pthread_mutex_unlock(&wctx->lock);
}

void pa_context_set_subscribe_callback(pa_context *c, pa_context_subscribe_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return;
    pthread_mutex_lock(&wctx->lock);
    wctx->sub_cb = cb;
    wctx->sub_ud = userdata;
    pthread_mutex_unlock(&wctx->lock);
}

pa_context_state_t pa_context_get_state(const pa_context *c) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return PA_CONTEXT_FAILED;
    pthread_mutex_lock(&wctx->lock);
    pa_context_state_t st;
    if (wctx->ever_ready && !wctx->user_disconnected && wctx->reconnecting) {
        st = PA_CONTEXT_READY;
    } else if (wctx->real_ctx) {
        st = real_pa_context_get_state(wctx->real_ctx);
        if (wctx->ever_ready && !wctx->user_disconnected &&
            (st == PA_CONTEXT_CONNECTING || st == PA_CONTEXT_AUTHORIZING || st == PA_CONTEXT_SETTING_NAME)) {
            st = PA_CONTEXT_READY;
        }
    } else {
        st = wctx->reported_state;
    }
    pthread_mutex_unlock(&wctx->lock);
    return st;
}

int pa_context_errno(const pa_context *c) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return PA_ERR_INVALID;
    pthread_mutex_lock(&wctx->lock);
    int e = (wctx->ever_ready && wctx->reconnecting) ? PA_OK :
            (wctx->real_ctx ? real_pa_context_errno(wctx->real_ctx) : wctx->last_errno);
    pthread_mutex_unlock(&wctx->lock);
    return e;
}

int pa_context_is_pending(const pa_context *c) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return real_pa_context_is_pending(c);
    pthread_mutex_lock(&wctx->lock);
    int r = wctx->real_ctx ? real_pa_context_is_pending(wctx->real_ctx) : 0;
    pthread_mutex_unlock(&wctx->lock);
    return r;
}

int pa_context_connect(pa_context *c, const char *server, pa_context_flags_t flags, const pa_spawn_api *api) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return -1;
    pthread_mutex_lock(&wctx->lock);
    free(wctx->server);
    wctx->server = xstrdup(server);
    wctx->server_set = (server != NULL);
    wctx->flags = flags;
    if (api) {
        wctx->spawn_api = *api;
        wctx->has_spawn_api = true;
    } else {
        wctx->has_spawn_api = false;
    }
    wctx->user_connected = true;
    wctx->user_disconnected = false;
    wctx->reported_state = PA_CONTEXT_CONNECTING;

    int rc = -1;
    if (wctx->real_ctx) {
        rc = real_pa_context_connect(wctx->real_ctx, server, (pa_context_flags_t)(flags & ~PA_CONTEXT_NOFAIL), api);
        if (rc < 0) {
            wctx->reconnecting = true;
            schedule_reconnect_timer(wctx, 150000);
            rc = 0;
        }
    }
    pthread_mutex_unlock(&wctx->lock);
    return rc;
}

void pa_context_disconnect(pa_context *c) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return;
    pa_context_ref(c);
    pthread_mutex_lock(&wctx->lock);
    wctx->user_disconnected = true;
    wctx->reconnecting = false;
    wctx->reported_state = PA_CONTEXT_TERMINATED;
    if (wctx->reconnect_timer && wctx->api) {
        wctx->api->time_free(wctx->reconnect_timer);
        wctx->reconnect_timer = NULL;
    }
    if (wctx->refresh_defer && wctx->api) {
        wctx->api->defer_free(wctx->refresh_defer);
        wctx->refresh_defer = NULL;
    }
    dead_handle_t *dead = detach_dead_handles_locked(wctx);
    pa_context *rc = wctx->real_ctx;
    pa_context_notify_cb_t cb = wctx->state_cb;
    void *ud = wctx->state_ud;
    pthread_mutex_unlock(&wctx->lock);
    flush_dead_handle_list(dead);
    if (rc) {
        real_pa_context_disconnect(rc);
    } else if (cb) {
        cb(c, ud);
    }
    pa_context_unref(c);
}

int pa_context_is_local(const pa_context *c) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return -1;
    pthread_mutex_lock(&wctx->lock);
    int r = wctx->real_ctx ? real_pa_context_is_local(wctx->real_ctx) : 1;
    pthread_mutex_unlock(&wctx->lock);
    return r;
}

const char *pa_context_get_server(const pa_context *c) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    const char *r = wctx->real_ctx ? real_pa_context_get_server(wctx->real_ctx) : wctx->server;
    pthread_mutex_unlock(&wctx->lock);
    return r;
}

uint32_t pa_context_get_protocol_version(const pa_context *c) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return PA_PROTOCOL_VERSION;
    return wctx->real_ctx ? real_pa_context_get_protocol_version(wctx->real_ctx) : PA_PROTOCOL_VERSION;
}

uint32_t pa_context_get_server_protocol_version(const pa_context *c) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return PA_PROTOCOL_VERSION;
    return wctx->real_ctx ? real_pa_context_get_server_protocol_version(wctx->real_ctx) : PA_PROTOCOL_VERSION;
}

uint32_t pa_context_get_index(const pa_context *c) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return PA_INVALID_INDEX;
    return wctx->real_ctx ? real_pa_context_get_index(wctx->real_ctx) : 0;
}

pa_time_event *pa_context_rttime_new(const pa_context *c, pa_usec_t usec, pa_time_event_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return real_pa_context_rttime_new(c, usec, cb, userdata);
    if (!wctx->real_ctx) return NULL;
    return real_pa_context_rttime_new(wctx->real_ctx, usec, cb, userdata);
}

void pa_context_rttime_restart(const pa_context *c, pa_time_event *e, pa_usec_t usec) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) {
        real_pa_context_rttime_restart(c, e, usec);
        return;
    }
    if (!wctx->real_ctx) return;
    real_pa_context_rttime_restart(wctx->real_ctx, e, usec);
}

size_t pa_context_get_tile_size(const pa_context *c, const pa_sample_spec *ss) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx) || !wctx->real_ctx) return 65536;
    return real_pa_context_get_tile_size(wctx->real_ctx, ss);
}

int pa_context_load_cookie_from_file(pa_context *c, const char *cookie_file_path) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx) || !wctx->real_ctx) return -1;
    return real_pa_context_load_cookie_from_file(wctx->real_ctx, cookie_file_path);
}

/* ============================================================================
 * Tee'd Introspection (Cache on live queries, serve cache on reconnect)
 * ============================================================================ */

typedef enum {
    TEE_KIND_SERVER = 0,
    TEE_KIND_SINK_LIST,
    TEE_KIND_SOURCE_LIST
} tee_kind_t;

struct tee_req {
    tee_kind_t kind;
    wctx_t *wctx;
    pa_context *owner_rc;
    void *user_cb;
    void *user_data;
    pa_sink_info *sinks;
    size_t n_sinks;
    size_t cap_sinks;
    pa_source_info *sources;
    size_t n_sources;
    size_t cap_sources;
    bool replayed;
    struct tee_req *global_next;
};

static pthread_mutex_t g_tee_lock = PTHREAD_MUTEX_INITIALIZER;
static tee_req_t *g_tee_head = NULL;

static pa_cache_t *pick_cache_locked(wctx_t *wctx) {
    if (wctx && (wctx->cache.has_server || wctx->cache.n_sinks > 0 || wctx->cache.n_sources > 0)) {
        return &wctx->cache;
    }
    return &g_global_cache;
}

static void tee_free(tee_req_t *tr) {
    if (!tr) return;
    for (size_t k = 0; k < tr->n_sinks; k++) free_sink_entry(&tr->sinks[k]);
    free(tr->sinks);
    for (size_t k = 0; k < tr->n_sources; k++) free_source_entry(&tr->sources[k]);
    free(tr->sources);
    free(tr);
}

static void tee_register(tee_req_t *tr) {
    if (!tr) return;
    pthread_mutex_lock(&g_tee_lock);
    tr->global_next = g_tee_head;
    g_tee_head = tr;
    pthread_mutex_unlock(&g_tee_lock);
}

static void tee_unregister(tee_req_t *tr) {
    if (!tr) return;
    pthread_mutex_lock(&g_tee_lock);
    tee_req_t **pp = &g_tee_head;
    while (*pp) {
        if (*pp == tr) {
            *pp = tr->global_next;
            tr->global_next = NULL;
            break;
        }
        pp = &(*pp)->global_next;
    }
    pthread_mutex_unlock(&g_tee_lock);
}

static bool tee_begin_cb(tee_req_t *tr, bool terminal, wctx_t **out_wctx, bool *out_replayed, void **out_cb, void **out_ud) {
    bool found = false;
    pthread_mutex_lock(&g_tee_lock);
    tee_req_t **pp = &g_tee_head;
    while (*pp) {
        if (*pp == tr) {
            found = true;
            *out_wctx = tr->wctx;
            *out_replayed = tr->replayed;
            *out_cb = tr->user_cb;
            *out_ud = tr->user_data;
            if (terminal) {
                *pp = tr->global_next;
                tr->global_next = NULL;
            }
            break;
        }
        pp = &(*pp)->global_next;
    }
    pthread_mutex_unlock(&g_tee_lock);
    return found;
}

static void detach_tees_for_wctx(wctx_t *wctx) {
    if (!wctx) return;
    pthread_mutex_lock(&g_tee_lock);
    for (tee_req_t *tr = g_tee_head; tr; tr = tr->global_next) {
        if (tr->wctx == wctx) {
            tr->wctx = NULL;
            tr->user_cb = NULL;
        }
    }
    pthread_mutex_unlock(&g_tee_lock);
}

static void reap_tees_for_rc(pa_context *rc) {
    if (!rc) return;
    tee_req_t *to_free = NULL;
    pthread_mutex_lock(&g_tee_lock);
    tee_req_t **pp = &g_tee_head;
    while (*pp) {
        tee_req_t *tr = *pp;
        if (tr->owner_rc == rc) {
            *pp = tr->global_next;
            tr->global_next = to_free;
            to_free = tr;
        } else {
            pp = &tr->global_next;
        }
    }
    pthread_mutex_unlock(&g_tee_lock);
    while (to_free) {
        tee_req_t *nx = to_free->global_next;
        tee_free(to_free);
        to_free = nx;
    }
}

typedef struct replay_item {
    tee_kind_t kind;
    void *user_cb;
    void *user_data;
    bool partial_sent;
    struct replay_item *next;
} replay_item_t;

static void replay_inflight_tees(wctx_t *wctx, pa_context *rc) {
    if (!wctx || !rc) return;
    replay_item_t *items = NULL;
    replay_item_t **tail = &items;

    pthread_mutex_lock(&g_tee_lock);
    for (tee_req_t *tr = g_tee_head; tr; tr = tr->global_next) {
        if (tr->wctx == wctx && tr->owner_rc == rc && !tr->replayed) {
            tr->replayed = true;
            if (tr->user_cb) {
                replay_item_t *ri = calloc(1, sizeof(replay_item_t));
                if (ri) {
                    ri->kind = tr->kind;
                    ri->user_cb = tr->user_cb;
                    ri->user_data = tr->user_data;
                    ri->partial_sent = (tr->kind == TEE_KIND_SINK_LIST && tr->n_sinks > 0) ||
                                       (tr->kind == TEE_KIND_SOURCE_LIST && tr->n_sources > 0);
                    *tail = ri;
                    tail = &ri->next;
                }
            }
        }
    }
    pthread_mutex_unlock(&g_tee_lock);

    while (items) {
        replay_item_t *ri = items;
        items = ri->next;
        if (ri->user_cb && is_valid_wctx(wctx) && !wctx->user_disconnected) {
            pthread_mutex_lock(&g_global_cache_lock);
            pa_cache_t *c = pick_cache_locked(wctx);
            switch (ri->kind) {
            case TEE_KIND_SERVER: {
                const pa_server_info *si = c->has_server ? &c->server : NULL;
                ((pa_server_info_cb_t)ri->user_cb)((pa_context *)wctx, si, ri->user_data);
                break;
            }
            case TEE_KIND_SINK_LIST: {
                pa_sink_info_cb_t cb = (pa_sink_info_cb_t)ri->user_cb;
                if (!ri->partial_sent) {
                    for (size_t k = 0; k < c->n_sinks; k++) {
                        cb((pa_context *)wctx, &c->sinks[k], 0, ri->user_data);
                    }
                }
                cb((pa_context *)wctx, NULL, 1, ri->user_data);
                break;
            }
            case TEE_KIND_SOURCE_LIST: {
                pa_source_info_cb_t cb = (pa_source_info_cb_t)ri->user_cb;
                if (!ri->partial_sent) {
                    for (size_t k = 0; k < c->n_sources; k++) {
                        cb((pa_context *)wctx, &c->sources[k], 0, ri->user_data);
                    }
                }
                cb((pa_context *)wctx, NULL, 1, ri->user_data);
                break;
            }
            }
            pthread_mutex_unlock(&g_global_cache_lock);
        }
        free(ri);
    }
}

static void tee_server_info_cb(pa_context *rc, const pa_server_info *i, void *ud) {
    (void)rc;
    tee_req_t *tr = (tee_req_t *)ud;
    wctx_t *wctx = NULL;
    bool already_replayed = false;
    void *ucb = NULL;
    void *uud = NULL;
    if (!tee_begin_cb(tr, true, &wctx, &already_replayed, &ucb, &uud)) {
        return;
    }
    if (already_replayed || !is_valid_wctx(wctx)) {
        tee_free(tr);
        return;
    }
    pa_context_ref((pa_context *)wctx);
    if (i) {
        pthread_mutex_lock(&wctx->lock);
        cache_set_server(&wctx->cache, i);
        pthread_mutex_unlock(&wctx->lock);

        pthread_mutex_lock(&g_global_cache_lock);
        cache_set_server(&g_global_cache, i);
        pthread_mutex_unlock(&g_global_cache_lock);
        if (ucb) {
            ((pa_server_info_cb_t)ucb)((pa_context *)wctx, i, uud);
        }
    } else if (!wctx->user_disconnected) {
        pthread_mutex_lock(&g_global_cache_lock);
        pa_cache_t *c = pick_cache_locked(wctx);
        const pa_server_info *si = c->has_server ? &c->server : NULL;
        if (ucb) {
            ((pa_server_info_cb_t)ucb)((pa_context *)wctx, si, uud);
        }
        pthread_mutex_unlock(&g_global_cache_lock);
    } else if (ucb) {
        ((pa_server_info_cb_t)ucb)((pa_context *)wctx, i, uud);
    }
    pa_context_unref((pa_context *)wctx);
    tee_free(tr);
}

static void tee_sink_list_cb(pa_context *rc, const pa_sink_info *i, int eol, void *ud) {
    (void)rc;
    tee_req_t *tr = (tee_req_t *)ud;
    wctx_t *wctx = NULL;
    bool already_replayed = false;
    void *ucb = NULL;
    void *uud = NULL;

    if (!eol && i) {
        if (!tee_begin_cb(tr, false, &wctx, &already_replayed, &ucb, &uud)) {
            return;
        }
        if (already_replayed || !is_valid_wctx(wctx)) {
            return;
        }
        if (tr->n_sinks == tr->cap_sinks) {
            size_t nc = tr->cap_sinks ? tr->cap_sinks * 2 : 8;
            pa_sink_info *nb = realloc(tr->sinks, nc * sizeof(pa_sink_info));
            if (nb) { tr->sinks = nb; tr->cap_sinks = nc; }
        }
        if (tr->n_sinks < tr->cap_sinks) {
            copy_sink_entry(&tr->sinks[tr->n_sinks++], i);
        }
        if (ucb) {
            pa_context_ref((pa_context *)wctx);
            ((pa_sink_info_cb_t)ucb)((pa_context *)wctx, i, eol, uud);
            pa_context_unref((pa_context *)wctx);
        }
        return;
    }

    if (!tee_begin_cb(tr, true, &wctx, &already_replayed, &ucb, &uud)) {
        return;
    }
    if (already_replayed || !is_valid_wctx(wctx)) {
        tee_free(tr);
        return;
    }

    pa_context_ref((pa_context *)wctx);
    if (eol < 0 && tr->n_sinks == 0 && !wctx->user_disconnected) {
        pthread_mutex_lock(&g_global_cache_lock);
        pa_cache_t *c = pick_cache_locked(wctx);
        if (c->n_sinks > 0) {
            if (ucb) {
                pa_sink_info_cb_t cb = (pa_sink_info_cb_t)ucb;
                for (size_t k = 0; k < c->n_sinks; k++) {
                    cb((pa_context *)wctx, &c->sinks[k], 0, uud);
                }
                cb((pa_context *)wctx, NULL, 1, uud);
            }
            pthread_mutex_unlock(&g_global_cache_lock);
        } else {
            pthread_mutex_unlock(&g_global_cache_lock);
            if (ucb) {
                int out_eol = wctx->reconnecting ? 1 : eol;
                ((pa_sink_info_cb_t)ucb)((pa_context *)wctx, i, out_eol, uud);
            }
        }
    } else {
        if (ucb) {
            int out_eol = (eol < 0 && (tr->n_sinks > 0 || wctx->reconnecting) && !wctx->user_disconnected) ? 1 : eol;
            ((pa_sink_info_cb_t)ucb)((pa_context *)wctx, i, out_eol, uud);
        }
    }

    if (eol > 0 && tr->n_sinks > 0 && is_valid_wctx(wctx)) {
        pthread_mutex_lock(&wctx->lock);
        cache_replace_sinks(&wctx->cache, tr->sinks, tr->n_sinks);
        tr->sinks = NULL;
        tr->n_sinks = 0;
        pthread_mutex_unlock(&wctx->lock);

        pa_sink_info *gcopy = calloc(wctx->cache.n_sinks, sizeof(pa_sink_info));
        if (gcopy) {
            for (size_t k = 0; k < wctx->cache.n_sinks; k++) copy_sink_entry(&gcopy[k], &wctx->cache.sinks[k]);
            pthread_mutex_lock(&g_global_cache_lock);
            cache_replace_sinks(&g_global_cache, gcopy, wctx->cache.n_sinks);
            pthread_mutex_unlock(&g_global_cache_lock);
        }
    }
    pa_context_unref((pa_context *)wctx);
    tee_free(tr);
}

static void tee_source_list_cb(pa_context *rc, const pa_source_info *i, int eol, void *ud) {
    (void)rc;
    tee_req_t *tr = (tee_req_t *)ud;
    wctx_t *wctx = NULL;
    bool already_replayed = false;
    void *ucb = NULL;
    void *uud = NULL;

    if (!eol && i) {
        if (!tee_begin_cb(tr, false, &wctx, &already_replayed, &ucb, &uud)) {
            return;
        }
        if (already_replayed || !is_valid_wctx(wctx)) {
            return;
        }
        if (tr->n_sources == tr->cap_sources) {
            size_t nc = tr->cap_sources ? tr->cap_sources * 2 : 8;
            pa_source_info *nb = realloc(tr->sources, nc * sizeof(pa_source_info));
            if (nb) { tr->sources = nb; tr->cap_sources = nc; }
        }
        if (tr->n_sources < tr->cap_sources) {
            copy_source_entry(&tr->sources[tr->n_sources++], i);
        }
        if (ucb) {
            pa_context_ref((pa_context *)wctx);
            ((pa_source_info_cb_t)ucb)((pa_context *)wctx, i, eol, uud);
            pa_context_unref((pa_context *)wctx);
        }
        return;
    }

    if (!tee_begin_cb(tr, true, &wctx, &already_replayed, &ucb, &uud)) {
        return;
    }
    if (already_replayed || !is_valid_wctx(wctx)) {
        tee_free(tr);
        return;
    }

    pa_context_ref((pa_context *)wctx);
    if (eol < 0 && tr->n_sources == 0 && !wctx->user_disconnected) {
        pthread_mutex_lock(&g_global_cache_lock);
        pa_cache_t *c = pick_cache_locked(wctx);
        if (c->n_sources > 0) {
            if (ucb) {
                pa_source_info_cb_t cb = (pa_source_info_cb_t)ucb;
                for (size_t k = 0; k < c->n_sources; k++) {
                    cb((pa_context *)wctx, &c->sources[k], 0, uud);
                }
                cb((pa_context *)wctx, NULL, 1, uud);
            }
            pthread_mutex_unlock(&g_global_cache_lock);
        } else {
            pthread_mutex_unlock(&g_global_cache_lock);
            if (ucb) {
                int out_eol = wctx->reconnecting ? 1 : eol;
                ((pa_source_info_cb_t)ucb)((pa_context *)wctx, i, out_eol, uud);
            }
        }
    } else {
        if (ucb) {
            int out_eol = (eol < 0 && (tr->n_sources > 0 || wctx->reconnecting) && !wctx->user_disconnected) ? 1 : eol;
            ((pa_source_info_cb_t)ucb)((pa_context *)wctx, i, out_eol, uud);
        }
    }

    if (eol > 0 && tr->n_sources > 0 && is_valid_wctx(wctx)) {
        pthread_mutex_lock(&wctx->lock);
        cache_replace_sources(&wctx->cache, tr->sources, tr->n_sources);
        tr->sources = NULL;
        tr->n_sources = 0;
        pthread_mutex_unlock(&wctx->lock);

        pa_source_info *gcopy = calloc(wctx->cache.n_sources, sizeof(pa_source_info));
        if (gcopy) {
            for (size_t k = 0; k < wctx->cache.n_sources; k++) copy_source_entry(&gcopy[k], &wctx->cache.sources[k]);
            pthread_mutex_lock(&g_global_cache_lock);
            cache_replace_sources(&g_global_cache, gcopy, wctx->cache.n_sources);
            pthread_mutex_unlock(&g_global_cache_lock);
        }
    }
    pa_context_unref((pa_context *)wctx);
    tee_free(tr);
}

/* Generic single-shot callback wrappers so callbacks receive wctx instead of real_ctx */
typedef struct {
    wctx_t *wctx;
    void *cb;
    void *ud;
} ctx_cb_wrap_t;

static void wrap_ctx_success_cb(pa_context *rc, int success, void *ud) {
    (void)rc;
    ctx_cb_wrap_t *w = (ctx_cb_wrap_t *)ud;
    if (is_valid_wctx(w->wctx)) {
        pa_context_ref((pa_context *)w->wctx);
        int out = (!success && w->wctx->reconnecting && !w->wctx->user_disconnected) ? 1 : success;
        if (w->cb) ((pa_context_success_cb_t)w->cb)((pa_context *)w->wctx, out, w->ud);
        pa_context_unref((pa_context *)w->wctx);
    }
    free(w);
}

static void wrap_ctx_notify_cb(pa_context *rc, void *ud) {
    (void)rc;
    ctx_cb_wrap_t *w = (ctx_cb_wrap_t *)ud;
    if (is_valid_wctx(w->wctx)) {
        pa_context_ref((pa_context *)w->wctx);
        if (w->cb) ((pa_context_notify_cb_t)w->cb)((pa_context *)w->wctx, w->ud);
        pa_context_unref((pa_context *)w->wctx);
    }
    free(w);
}

static void wrap_ctx_index_cb(pa_context *rc, uint32_t idx, void *ud) {
    (void)rc;
    ctx_cb_wrap_t *w = (ctx_cb_wrap_t *)ud;
    if (is_valid_wctx(w->wctx)) {
        pa_context_ref((pa_context *)w->wctx);
        if (w->cb) ((pa_context_index_cb_t)w->cb)((pa_context *)w->wctx, idx, w->ud);
        pa_context_unref((pa_context *)w->wctx);
    }
    free(w);
}

static void wrap_ctx_string_cb(pa_context *rc, int success, char *response, void *ud) {
    (void)rc;
    ctx_cb_wrap_t *w = (ctx_cb_wrap_t *)ud;
    if (is_valid_wctx(w->wctx)) {
        pa_context_ref((pa_context *)w->wctx);
        if (w->cb) ((pa_context_string_cb_t)w->cb)((pa_context *)w->wctx, success, response, w->ud);
        pa_context_unref((pa_context *)w->wctx);
    }
    free(w);
}

static void wrap_ctx_stat_cb(pa_context *rc, const pa_stat_info *i, void *ud) {
    (void)rc;
    ctx_cb_wrap_t *w = (ctx_cb_wrap_t *)ud;
    if (is_valid_wctx(w->wctx)) {
        pa_context_ref((pa_context *)w->wctx);
        if (w->cb) ((pa_stat_info_cb_t)w->cb)((pa_context *)w->wctx, i, w->ud);
        pa_context_unref((pa_context *)w->wctx);
    }
    free(w);
}

#define DEFINE_WRAP_CTX_EOL_CB(fn_name, info_type, cb_type) \
static void fn_name(pa_context *rc, const info_type *i, int eol, void *ud) { \
    (void)rc; \
    ctx_cb_wrap_t *w = (ctx_cb_wrap_t *)ud; \
    if (is_valid_wctx(w->wctx)) { \
        pa_context_ref((pa_context *)w->wctx); \
        int out_eol = (eol < 0 && w->wctx->reconnecting && !w->wctx->user_disconnected) ? 1 : eol; \
        if (w->cb) ((cb_type)w->cb)((pa_context *)w->wctx, i, out_eol, w->ud); \
        pa_context_unref((pa_context *)w->wctx); \
    } \
    if (eol) free(w); \
}

DEFINE_WRAP_CTX_EOL_CB(wrap_ctx_client_cb, pa_client_info, pa_client_info_cb_t)
DEFINE_WRAP_CTX_EOL_CB(wrap_ctx_sink_cb, pa_sink_info, pa_sink_info_cb_t)
DEFINE_WRAP_CTX_EOL_CB(wrap_ctx_source_cb, pa_source_info, pa_source_info_cb_t)
DEFINE_WRAP_CTX_EOL_CB(wrap_ctx_sink_input_cb, pa_sink_input_info, pa_sink_input_info_cb_t)
DEFINE_WRAP_CTX_EOL_CB(wrap_ctx_source_output_cb, pa_source_output_info, pa_source_output_info_cb_t)
DEFINE_WRAP_CTX_EOL_CB(wrap_ctx_module_cb, pa_module_info, pa_module_info_cb_t)
DEFINE_WRAP_CTX_EOL_CB(wrap_ctx_card_cb, pa_card_info, pa_card_info_cb_t)
DEFINE_WRAP_CTX_EOL_CB(wrap_ctx_sample_cb, pa_sample_info, pa_sample_info_cb_t)
DEFINE_WRAP_CTX_EOL_CB(wrap_ctx_autoload_cb, pa_autoload_info, pa_autoload_info_cb_t)
DEFINE_WRAP_CTX_EOL_CB(wrap_ctx_ext_dev_restore_cb, pa_ext_device_restore_info, pa_ext_device_restore_read_device_formats_cb_t)
DEFINE_WRAP_CTX_EOL_CB(wrap_ctx_ext_dev_mgr_cb, pa_ext_device_manager_info, pa_ext_device_manager_read_cb_t)
DEFINE_WRAP_CTX_EOL_CB(wrap_ctx_ext_stream_restore_cb, pa_ext_stream_restore_info, pa_ext_stream_restore_read_cb_t)

static ctx_cb_wrap_t *mk_ctx_wrap(wctx_t *wctx, void *cb, void *ud) {
    ctx_cb_wrap_t *w = calloc(1, sizeof(ctx_cb_wrap_t));
    if (w) { w->wctx = wctx; w->cb = cb; w->ud = ud; }
    return w;
}

static bool ctx_is_live_ready(wctx_t *wctx) {
    return wctx->real_ctx && real_pa_context_get_state(wctx->real_ctx) == PA_CONTEXT_READY;
}

pa_operation *pa_context_subscribe(pa_context *c, pa_subscription_mask_t m, pa_context_success_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    wctx->has_subscribe = (m != PA_SUBSCRIPTION_MASK_NULL);
    wctx->sub_mask = m;
    if (!ctx_is_live_ready(wctx)) {
        pa_operation *op = wctx->reconnecting ? spawn_synth_op(wctx, SYNTH_KIND_SUCCESS, (void *)cb, userdata, NULL, 0) : NULL;
        pthread_mutex_unlock(&wctx->lock);
        return op;
    }
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata);
    pa_operation *op = real_pa_context_subscribe(wctx->real_ctx, m, w ? wrap_ctx_success_cb : NULL, w);
    if (!op) free(w);
    pthread_mutex_unlock(&wctx->lock);
    return op;
}

pa_operation *pa_context_get_server_info(pa_context *c, pa_server_info_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    if (!ctx_is_live_ready(wctx)) {
        pa_operation *op = wctx->reconnecting ? spawn_synth_op(wctx, SYNTH_KIND_SERVER_INFO, (void *)cb, userdata, NULL, 0) : NULL;
        pthread_mutex_unlock(&wctx->lock);
        return op;
    }
    tee_req_t *tr = calloc(1, sizeof(tee_req_t));
    if (!tr) { pthread_mutex_unlock(&wctx->lock); return NULL; }
    tr->kind = TEE_KIND_SERVER; tr->wctx = wctx; tr->owner_rc = wctx->real_ctx; tr->user_cb = (void *)cb; tr->user_data = userdata;
    tee_register(tr);
    pa_operation *op = real_pa_context_get_server_info(wctx->real_ctx, tee_server_info_cb, tr);
    if (!op) { tee_unregister(tr); free(tr); }
    pthread_mutex_unlock(&wctx->lock);
    return op;
}

pa_operation *pa_context_get_sink_info_list(pa_context *c, pa_sink_info_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    if (!ctx_is_live_ready(wctx)) {
        pa_operation *op = wctx->reconnecting ? spawn_synth_op(wctx, SYNTH_KIND_SINK_LIST, (void *)cb, userdata, NULL, 0) : NULL;
        pthread_mutex_unlock(&wctx->lock);
        return op;
    }
    tee_req_t *tr = calloc(1, sizeof(tee_req_t));
    if (!tr) { pthread_mutex_unlock(&wctx->lock); return NULL; }
    tr->kind = TEE_KIND_SINK_LIST; tr->wctx = wctx; tr->owner_rc = wctx->real_ctx; tr->user_cb = (void *)cb; tr->user_data = userdata;
    tee_register(tr);
    pa_operation *op = real_pa_context_get_sink_info_list(wctx->real_ctx, tee_sink_list_cb, tr);
    if (!op) { tee_unregister(tr); free(tr); }
    pthread_mutex_unlock(&wctx->lock);
    return op;
}

pa_operation *pa_context_get_sink_info_by_name(pa_context *c, const char *name, pa_sink_info_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    if (!ctx_is_live_ready(wctx)) {
        pa_operation *op = wctx->reconnecting ? spawn_synth_op(wctx, SYNTH_KIND_SINK_BY_NAME, (void *)cb, userdata, name, 0) : NULL;
        pthread_mutex_unlock(&wctx->lock);
        return op;
    }
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata);
    pa_operation *op = real_pa_context_get_sink_info_by_name(wctx->real_ctx, name, w ? wrap_ctx_sink_cb : NULL, w);
    if (!op) free(w);
    pthread_mutex_unlock(&wctx->lock);
    return op;
}

pa_operation *pa_context_get_sink_info_by_index(pa_context *c, uint32_t idx, pa_sink_info_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    if (!ctx_is_live_ready(wctx)) {
        pa_operation *op = wctx->reconnecting ? spawn_synth_op(wctx, SYNTH_KIND_SINK_BY_INDEX, (void *)cb, userdata, NULL, idx) : NULL;
        pthread_mutex_unlock(&wctx->lock);
        return op;
    }
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata);
    pa_operation *op = real_pa_context_get_sink_info_by_index(wctx->real_ctx, idx, w ? wrap_ctx_sink_cb : NULL, w);
    if (!op) free(w);
    pthread_mutex_unlock(&wctx->lock);
    return op;
}

pa_operation *pa_context_get_source_info_list(pa_context *c, pa_source_info_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    if (!ctx_is_live_ready(wctx)) {
        pa_operation *op = wctx->reconnecting ? spawn_synth_op(wctx, SYNTH_KIND_SOURCE_LIST, (void *)cb, userdata, NULL, 0) : NULL;
        pthread_mutex_unlock(&wctx->lock);
        return op;
    }
    tee_req_t *tr = calloc(1, sizeof(tee_req_t));
    if (!tr) { pthread_mutex_unlock(&wctx->lock); return NULL; }
    tr->kind = TEE_KIND_SOURCE_LIST; tr->wctx = wctx; tr->owner_rc = wctx->real_ctx; tr->user_cb = (void *)cb; tr->user_data = userdata;
    tee_register(tr);
    pa_operation *op = real_pa_context_get_source_info_list(wctx->real_ctx, tee_source_list_cb, tr);
    if (!op) { tee_unregister(tr); free(tr); }
    pthread_mutex_unlock(&wctx->lock);
    return op;
}

pa_operation *pa_context_get_source_info_by_name(pa_context *c, const char *name, pa_source_info_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    if (!ctx_is_live_ready(wctx)) {
        pa_operation *op = wctx->reconnecting ? spawn_synth_op(wctx, SYNTH_KIND_SOURCE_BY_NAME, (void *)cb, userdata, name, 0) : NULL;
        pthread_mutex_unlock(&wctx->lock);
        return op;
    }
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata);
    pa_operation *op = real_pa_context_get_source_info_by_name(wctx->real_ctx, name, w ? wrap_ctx_source_cb : NULL, w);
    if (!op) free(w);
    pthread_mutex_unlock(&wctx->lock);
    return op;
}

pa_operation *pa_context_get_source_info_by_index(pa_context *c, uint32_t idx, pa_source_info_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    if (!ctx_is_live_ready(wctx)) {
        pa_operation *op = wctx->reconnecting ? spawn_synth_op(wctx, SYNTH_KIND_SOURCE_BY_INDEX, (void *)cb, userdata, NULL, idx) : NULL;
        pthread_mutex_unlock(&wctx->lock);
        return op;
    }
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata);
    pa_operation *op = real_pa_context_get_source_info_by_index(wctx->real_ctx, idx, w ? wrap_ctx_source_cb : NULL, w);
    if (!op) free(w);
    pthread_mutex_unlock(&wctx->lock);
    return op;
}

/* Macro for remaining pa_context_* async ops */
#define CTX_OP_SUCCESS(fname, params, fwd_args) \
pa_operation *fname params { \
    ensure_init(); \
    wctx_t *wctx = (wctx_t *)c; \
    if (!is_valid_wctx(wctx)) return NULL; \
    pthread_mutex_lock(&wctx->lock); \
    if (!ctx_is_live_ready(wctx)) { \
        pa_operation *op = wctx->reconnecting ? spawn_synth_op(wctx, SYNTH_KIND_SUCCESS, (void *)cb, userdata, NULL, 0) : NULL; \
        pthread_mutex_unlock(&wctx->lock); \
        return op; \
    } \
    if (!real_##fname) { \
        pthread_mutex_unlock(&wctx->lock); \
        return NULL; \
    } \
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata); \
    pa_operation *op = real_##fname fwd_args; \
    if (!op) free(w); \
    pthread_mutex_unlock(&wctx->lock); \
    return op; \
}

#define CTX_OP_LIST(fname, params, fwd_args, wrap_cb) \
pa_operation *fname params { \
    ensure_init(); \
    wctx_t *wctx = (wctx_t *)c; \
    if (!is_valid_wctx(wctx)) return NULL; \
    pthread_mutex_lock(&wctx->lock); \
    if (!ctx_is_live_ready(wctx)) { \
        pa_operation *op = wctx->reconnecting ? spawn_synth_op(wctx, SYNTH_KIND_EMPTY_EOL, (void *)cb, userdata, NULL, 0) : NULL; \
        pthread_mutex_unlock(&wctx->lock); \
        return op; \
    } \
    if (!real_##fname) { \
        pthread_mutex_unlock(&wctx->lock); \
        return NULL; \
    } \
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata); \
    pa_operation *op = real_##fname fwd_args; \
    if (!op) free(w); \
    pthread_mutex_unlock(&wctx->lock); \
    return op; \
}

pa_operation *pa_context_drain(pa_context *c, pa_context_notify_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    if (!wctx->real_ctx) {
        pthread_mutex_unlock(&wctx->lock);
        return NULL;
    }
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata);
    pa_operation *op = real_pa_context_drain(wctx->real_ctx, w ? wrap_ctx_notify_cb : NULL, w);
    if (!op) free(w);
    pthread_mutex_unlock(&wctx->lock);
    return op;
}

CTX_OP_SUCCESS(pa_context_exit_daemon, (pa_context *c, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_default_sink, (pa_context *c, const char *name, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, name, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_default_source, (pa_context *c, const char *name, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, name, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_name, (pa_context *c, const char *name, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, name, w ? wrap_ctx_success_cb : NULL, w))
pa_operation *pa_context_proplist_update(pa_context *c, pa_update_mode_t mode, const pa_proplist *p, pa_context_success_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return c ? real_pa_context_proplist_update(c, mode, p, cb, userdata) : NULL;
    pthread_mutex_lock(&wctx->lock);
    if (!ctx_is_live_ready(wctx)) {
        pa_operation *op = wctx->reconnecting ? spawn_synth_op(wctx, SYNTH_KIND_SUCCESS, (void *)cb, userdata, NULL, 0) : NULL;
        pthread_mutex_unlock(&wctx->lock);
        return op;
    }
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata);
    pa_operation *op = real_pa_context_proplist_update(wctx->real_ctx, mode, p, w ? wrap_ctx_success_cb : NULL, w);
    if (!op) free(w);
    pthread_mutex_unlock(&wctx->lock);
    return op;
}
CTX_OP_SUCCESS(pa_context_proplist_remove, (pa_context *c, const char *const keys[], pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, keys, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_sink_volume_by_index, (pa_context *c, uint32_t idx, const pa_cvolume *volume, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, volume, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_sink_volume_by_name, (pa_context *c, const char *name, const pa_cvolume *volume, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, name, volume, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_sink_mute_by_index, (pa_context *c, uint32_t idx, int mute, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, mute, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_sink_mute_by_name, (pa_context *c, const char *name, int mute, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, name, mute, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_suspend_sink_by_name, (pa_context *c, const char *sink_name, int suspend, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, sink_name, suspend, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_suspend_sink_by_index, (pa_context *c, uint32_t idx, int suspend, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, suspend, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_sink_port_by_index, (pa_context *c, uint32_t idx, const char *port, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, port, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_sink_port_by_name, (pa_context *c, const char *name, const char *port, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, name, port, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_source_volume_by_index, (pa_context *c, uint32_t idx, const pa_cvolume *volume, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, volume, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_source_volume_by_name, (pa_context *c, const char *name, const pa_cvolume *volume, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, name, volume, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_source_mute_by_index, (pa_context *c, uint32_t idx, int mute, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, mute, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_source_mute_by_name, (pa_context *c, const char *name, int mute, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, name, mute, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_suspend_source_by_name, (pa_context *c, const char *source_name, int suspend, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, source_name, suspend, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_suspend_source_by_index, (pa_context *c, uint32_t idx, int suspend, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, suspend, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_source_port_by_index, (pa_context *c, uint32_t idx, const char *port, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, port, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_source_port_by_name, (pa_context *c, const char *name, const char *port, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, name, port, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_unload_module, (pa_context *c, uint32_t idx, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_kill_client, (pa_context *c, uint32_t idx, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_card_profile_by_index, (pa_context *c, uint32_t idx, const char *profile, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, profile, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_card_profile_by_name, (pa_context *c, const char *name, const char *profile, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, name, profile, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_port_latency_offset, (pa_context *c, const char *card_name, const char *port_name, int64_t offset, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, card_name, port_name, offset, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_move_sink_input_by_name, (pa_context *c, uint32_t idx, const char *sink_name, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, sink_name, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_move_sink_input_by_index, (pa_context *c, uint32_t idx, uint32_t sink_idx, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, sink_idx, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_sink_input_volume, (pa_context *c, uint32_t idx, const pa_cvolume *volume, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, volume, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_sink_input_mute, (pa_context *c, uint32_t idx, int mute, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, mute, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_kill_sink_input, (pa_context *c, uint32_t idx, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_move_source_output_by_name, (pa_context *c, uint32_t idx, const char *source_name, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, source_name, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_move_source_output_by_index, (pa_context *c, uint32_t idx, uint32_t source_idx, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, source_idx, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_source_output_volume, (pa_context *c, uint32_t idx, const pa_cvolume *volume, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, volume, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_set_source_output_mute, (pa_context *c, uint32_t idx, int mute, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, mute, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_kill_source_output, (pa_context *c, uint32_t idx, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_remove_sample, (pa_context *c, const char *name, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, name, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_play_sample, (pa_context *c, const char *name, const char *dev, pa_volume_t volume, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, name, dev, volume, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_remove_autoload_by_name, (pa_context *c, const char *name, pa_autoload_type_t type, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, name, type, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_context_remove_autoload_by_index, (pa_context *c, uint32_t idx, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, idx, w ? wrap_ctx_success_cb : NULL, w))

CTX_OP_LIST(pa_context_get_module_info, (pa_context *c, uint32_t idx, pa_module_info_cb_t cb, void *userdata), (wctx->real_ctx, idx, w ? wrap_ctx_module_cb : NULL, w), wrap_ctx_module_cb)
CTX_OP_LIST(pa_context_get_module_info_list, (pa_context *c, pa_module_info_cb_t cb, void *userdata), (wctx->real_ctx, w ? wrap_ctx_module_cb : NULL, w), wrap_ctx_module_cb)
CTX_OP_LIST(pa_context_get_client_info, (pa_context *c, uint32_t idx, pa_client_info_cb_t cb, void *userdata), (wctx->real_ctx, idx, w ? wrap_ctx_client_cb : NULL, w), wrap_ctx_client_cb)
CTX_OP_LIST(pa_context_get_client_info_list, (pa_context *c, pa_client_info_cb_t cb, void *userdata), (wctx->real_ctx, w ? wrap_ctx_client_cb : NULL, w), wrap_ctx_client_cb)
CTX_OP_LIST(pa_context_get_card_info_by_index, (pa_context *c, uint32_t idx, pa_card_info_cb_t cb, void *userdata), (wctx->real_ctx, idx, w ? wrap_ctx_card_cb : NULL, w), wrap_ctx_card_cb)
CTX_OP_LIST(pa_context_get_card_info_by_name, (pa_context *c, const char *name, pa_card_info_cb_t cb, void *userdata), (wctx->real_ctx, name, w ? wrap_ctx_card_cb : NULL, w), wrap_ctx_card_cb)
CTX_OP_LIST(pa_context_get_card_info_list, (pa_context *c, pa_card_info_cb_t cb, void *userdata), (wctx->real_ctx, w ? wrap_ctx_card_cb : NULL, w), wrap_ctx_card_cb)
CTX_OP_LIST(pa_context_get_sink_input_info, (pa_context *c, uint32_t idx, pa_sink_input_info_cb_t cb, void *userdata), (wctx->real_ctx, idx, w ? wrap_ctx_sink_input_cb : NULL, w), wrap_ctx_sink_input_cb)
CTX_OP_LIST(pa_context_get_sink_input_info_list, (pa_context *c, pa_sink_input_info_cb_t cb, void *userdata), (wctx->real_ctx, w ? wrap_ctx_sink_input_cb : NULL, w), wrap_ctx_sink_input_cb)
CTX_OP_LIST(pa_context_get_source_output_info, (pa_context *c, uint32_t idx, pa_source_output_info_cb_t cb, void *userdata), (wctx->real_ctx, idx, w ? wrap_ctx_source_output_cb : NULL, w), wrap_ctx_source_output_cb)
CTX_OP_LIST(pa_context_get_source_output_info_list, (pa_context *c, pa_source_output_info_cb_t cb, void *userdata), (wctx->real_ctx, w ? wrap_ctx_source_output_cb : NULL, w), wrap_ctx_source_output_cb)
CTX_OP_LIST(pa_context_get_sample_info_by_name, (pa_context *c, const char *name, pa_sample_info_cb_t cb, void *userdata), (wctx->real_ctx, name, w ? wrap_ctx_sample_cb : NULL, w), wrap_ctx_sample_cb)
CTX_OP_LIST(pa_context_get_sample_info_by_index, (pa_context *c, uint32_t idx, pa_sample_info_cb_t cb, void *userdata), (wctx->real_ctx, idx, w ? wrap_ctx_sample_cb : NULL, w), wrap_ctx_sample_cb)
CTX_OP_LIST(pa_context_get_sample_info_list, (pa_context *c, pa_sample_info_cb_t cb, void *userdata), (wctx->real_ctx, w ? wrap_ctx_sample_cb : NULL, w), wrap_ctx_sample_cb)
CTX_OP_LIST(pa_context_get_autoload_info_by_name, (pa_context *c, const char *name, pa_autoload_type_t type, pa_autoload_info_cb_t cb, void *userdata), (wctx->real_ctx, name, type, w ? wrap_ctx_autoload_cb : NULL, w), wrap_ctx_autoload_cb)
CTX_OP_LIST(pa_context_get_autoload_info_by_index, (pa_context *c, uint32_t idx, pa_autoload_info_cb_t cb, void *userdata), (wctx->real_ctx, idx, w ? wrap_ctx_autoload_cb : NULL, w), wrap_ctx_autoload_cb)
CTX_OP_LIST(pa_context_get_autoload_info_list, (pa_context *c, pa_autoload_info_cb_t cb, void *userdata), (wctx->real_ctx, w ? wrap_ctx_autoload_cb : NULL, w), wrap_ctx_autoload_cb)

#define CTX_OP_TEST(fname, cb_type) \
pa_operation *fname(pa_context *c, cb_type cb, void *userdata) { \
    ensure_init(); \
    wctx_t *wctx = (wctx_t *)c; \
    if (!is_valid_wctx(wctx)) return NULL; \
    pthread_mutex_lock(&wctx->lock); \
    if (!ctx_is_live_ready(wctx) || !real_##fname) { \
        pthread_mutex_unlock(&wctx->lock); \
        return NULL; \
    } \
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata); \
    pa_operation *op = real_##fname(wctx->real_ctx, w ? (cb_type)wrap_ctx_index_cb : NULL, w); \
    if (!op) free(w); \
    pthread_mutex_unlock(&wctx->lock); \
    return op; \
}

#define CTX_EXT_SUBSCRIBE(fname, field_enabled) \
pa_operation *fname(pa_context *c, int enable, pa_context_success_cb_t cb, void *userdata) { \
    ensure_init(); \
    wctx_t *wctx = (wctx_t *)c; \
    if (!is_valid_wctx(wctx)) return NULL; \
    pthread_mutex_lock(&wctx->lock); \
    wctx->field_enabled = (enable != 0); \
    if (!ctx_is_live_ready(wctx)) { \
        pa_operation *op = wctx->reconnecting ? spawn_synth_op(wctx, SYNTH_KIND_SUCCESS, (void *)cb, userdata, NULL, 0) : NULL; \
        pthread_mutex_unlock(&wctx->lock); \
        return op; \
    } \
    if (!real_##fname) { \
        pthread_mutex_unlock(&wctx->lock); \
        return NULL; \
    } \
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata); \
    pa_operation *op = real_##fname(wctx->real_ctx, enable, w ? wrap_ctx_success_cb : NULL, w); \
    if (!op) free(w); \
    pthread_mutex_unlock(&wctx->lock); \
    return op; \
}

#define CTX_EXT_SET_SUB_CB(fname, cb_type, field_cb, field_ud, internal_cb) \
void fname(pa_context *c, cb_type cb, void *userdata) { \
    ensure_init(); \
    wctx_t *wctx = (wctx_t *)c; \
    if (!is_valid_wctx(wctx)) return; \
    pthread_mutex_lock(&wctx->lock); \
    wctx->field_cb = cb; \
    wctx->field_ud = userdata; \
    if (wctx->real_ctx && real_##fname) { \
        real_##fname(wctx->real_ctx, cb ? internal_cb : NULL, cb ? wctx : NULL); \
    } \
    pthread_mutex_unlock(&wctx->lock); \
}

CTX_OP_TEST(pa_ext_device_restore_test, pa_ext_device_restore_test_cb_t)
CTX_EXT_SUBSCRIBE(pa_ext_device_restore_subscribe, ext_dev_restore_sub_enabled)
CTX_EXT_SET_SUB_CB(pa_ext_device_restore_set_subscribe_cb, pa_ext_device_restore_subscribe_cb_t, ext_dev_restore_sub_cb, ext_dev_restore_sub_ud, internal_ext_dev_restore_sub_cb)
CTX_OP_LIST(pa_ext_device_restore_read_formats_all, (pa_context *c, pa_ext_device_restore_read_device_formats_cb_t cb, void *userdata), (wctx->real_ctx, w ? wrap_ctx_ext_dev_restore_cb : NULL, w), wrap_ctx_ext_dev_restore_cb)
CTX_OP_LIST(pa_ext_device_restore_read_formats, (pa_context *c, pa_device_type_t type, uint32_t idx, pa_ext_device_restore_read_device_formats_cb_t cb, void *userdata), (wctx->real_ctx, type, idx, w ? wrap_ctx_ext_dev_restore_cb : NULL, w), wrap_ctx_ext_dev_restore_cb)
CTX_OP_SUCCESS(pa_ext_device_restore_save_formats, (pa_context *c, pa_device_type_t type, uint32_t idx, uint8_t n_formats, pa_format_info **formats, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, type, idx, n_formats, formats, w ? wrap_ctx_success_cb : NULL, w))

CTX_OP_TEST(pa_ext_device_manager_test, pa_ext_device_manager_test_cb_t)
CTX_OP_LIST(pa_ext_device_manager_read, (pa_context *c, pa_ext_device_manager_read_cb_t cb, void *userdata), (wctx->real_ctx, w ? wrap_ctx_ext_dev_mgr_cb : NULL, w), wrap_ctx_ext_dev_mgr_cb)
CTX_OP_SUCCESS(pa_ext_device_manager_set_device_description, (pa_context *c, const char *device, const char *description, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, device, description, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_ext_device_manager_delete, (pa_context *c, const char *const s[], pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, s, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_ext_device_manager_enable_role_device_priority_routing, (pa_context *c, int enable, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, enable, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_ext_device_manager_reorder_devices_for_role, (pa_context *c, const char *role, const char **devices, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, role, devices, w ? wrap_ctx_success_cb : NULL, w))
CTX_EXT_SUBSCRIBE(pa_ext_device_manager_subscribe, ext_dev_mgr_sub_enabled)
CTX_EXT_SET_SUB_CB(pa_ext_device_manager_set_subscribe_cb, pa_ext_device_manager_subscribe_cb_t, ext_dev_mgr_sub_cb, ext_dev_mgr_sub_ud, internal_ext_dev_mgr_sub_cb)

CTX_OP_TEST(pa_ext_stream_restore_test, pa_ext_stream_restore_test_cb_t)
CTX_OP_LIST(pa_ext_stream_restore_read, (pa_context *c, pa_ext_stream_restore_read_cb_t cb, void *userdata), (wctx->real_ctx, w ? wrap_ctx_ext_stream_restore_cb : NULL, w), wrap_ctx_ext_stream_restore_cb)
CTX_OP_SUCCESS(pa_ext_stream_restore_write, (pa_context *c, pa_update_mode_t mode, const pa_ext_stream_restore_info data[], unsigned n, int apply_immediately, pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, mode, data, n, apply_immediately, w ? wrap_ctx_success_cb : NULL, w))
CTX_OP_SUCCESS(pa_ext_stream_restore_delete, (pa_context *c, const char *const s[], pa_context_success_cb_t cb, void *userdata), (wctx->real_ctx, s, w ? wrap_ctx_success_cb : NULL, w))
CTX_EXT_SUBSCRIBE(pa_ext_stream_restore_subscribe, ext_stream_restore_sub_enabled)
CTX_EXT_SET_SUB_CB(pa_ext_stream_restore_set_subscribe_cb, pa_ext_stream_restore_subscribe_cb_t, ext_stream_restore_sub_cb, ext_stream_restore_sub_ud, internal_ext_stream_restore_sub_cb)

pa_operation *pa_context_load_module(pa_context *c, const char *name, const char *argument, pa_context_index_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    if (!ctx_is_live_ready(wctx)) { pthread_mutex_unlock(&wctx->lock); return NULL; }
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata);
    pa_operation *op = real_pa_context_load_module(wctx->real_ctx, name, argument, w ? wrap_ctx_index_cb : NULL, w);
    if (!op) free(w);
    pthread_mutex_unlock(&wctx->lock);
    return op;
}

pa_operation *pa_context_add_autoload(pa_context *c, const char *name, pa_autoload_type_t type, const char *module, const char *argument, pa_context_index_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    if (!ctx_is_live_ready(wctx)) { pthread_mutex_unlock(&wctx->lock); return NULL; }
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata);
    pa_operation *op = real_pa_context_add_autoload(wctx->real_ctx, name, type, module, argument, w ? wrap_ctx_index_cb : NULL, w);
    if (!op) free(w);
    pthread_mutex_unlock(&wctx->lock);
    return op;
}

pa_operation *pa_context_play_sample_with_proplist(pa_context *c, const char *name, const char *dev, pa_volume_t volume, const pa_proplist *proplist, pa_context_index_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    if (!ctx_is_live_ready(wctx)) { pthread_mutex_unlock(&wctx->lock); return NULL; }
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata);
    pa_operation *op = real_pa_context_play_sample_with_proplist(wctx->real_ctx, name, dev, volume, proplist, w ? wrap_ctx_index_cb : NULL, w);
    if (!op) free(w);
    pthread_mutex_unlock(&wctx->lock);
    return op;
}

pa_operation *pa_context_send_message_to_object(pa_context *c, const char *recipient_name, const char *message, const char *message_parameters, pa_context_string_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    if (!ctx_is_live_ready(wctx)) { pthread_mutex_unlock(&wctx->lock); return NULL; }
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata);
    pa_operation *op = real_pa_context_send_message_to_object(wctx->real_ctx, recipient_name, message, message_parameters, w ? wrap_ctx_string_cb : NULL, w);
    if (!op) free(w);
    pthread_mutex_unlock(&wctx->lock);
    return op;
}

pa_operation *pa_context_stat(pa_context *c, pa_stat_info_cb_t cb, void *userdata) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pthread_mutex_lock(&wctx->lock);
    if (!ctx_is_live_ready(wctx)) { pthread_mutex_unlock(&wctx->lock); return NULL; }
    ctx_cb_wrap_t *w = mk_ctx_wrap(wctx, (void *)cb, userdata);
    pa_operation *op = real_pa_context_stat(wctx->real_ctx, w ? wrap_ctx_stat_cb : NULL, w);
    if (!op) free(w);
    pthread_mutex_unlock(&wctx->lock);
    return op;
}

/* ============================================================================
 * Stream Creation / Lifecycle / I/O
 * ============================================================================ */

static wstream_t *alloc_wstream(wctx_t *wctx, pa_stream *rs, const char *name,
                                const pa_sample_spec *ss, const pa_channel_map *map,
                                pa_proplist *p) {
    wstream_t *ws = calloc(1, sizeof(wstream_t));
    if (!ws) return NULL;
    atomic_init(&ws->ref, 1);
    ws->wctx = wctx;
    ws->real_stream = rs;
    ws->reported_state = PA_STREAM_UNCONNECTED;
    ws->name = xstrdup(name);
    if (ss) ws->ss = *ss;
    if (map) {
        ws->cmap = *map;
        ws->has_cmap = true;
    } else if (ss) {
        pa_channel_map_init_auto(&ws->cmap, ss->channels, PA_CHANNEL_MAP_DEFAULT);
        ws->has_cmap = true;
    }
    ws->proplist = p ? pa_proplist_copy(p) : NULL;
    ws->last_stream_index = PA_INVALID_INDEX;
    ws->last_device_index = PA_INVALID_INDEX;

    if (rs) real_pa_stream_set_state_callback(rs, internal_stream_state_cb, ws);

    pthread_mutex_lock(&wctx->lock);
    ws->next = wctx->streams;
    wctx->streams = ws;
    pthread_mutex_unlock(&wctx->lock);
    pthread_mutex_lock(&g_stream_lock);
    ws->global_next = g_stream_head;
    g_stream_head = ws;
    pthread_mutex_unlock(&g_stream_lock);
    pa_context_ref((pa_context *)wctx);
    return ws;
}

pa_stream *pa_stream_new_with_proplist(pa_context *c, const char *name, const pa_sample_spec *ss, const pa_channel_map *map, pa_proplist *p) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    if (!wctx->real_ctx) {
        if (!wctx->reconnecting || wctx->user_disconnected) return NULL;
        wstream_t *ws = alloc_wstream(wctx, NULL, name, ss, map, p);
        if (ws) ws->reconnecting = true;
        return (pa_stream *)ws;
    }
    pa_stream *rs = real_pa_stream_new_with_proplist(wctx->real_ctx, name, ss, map, p);
    if (!rs) return NULL;
    wstream_t *ws = alloc_wstream(wctx, rs, name, ss, map, p);
    if (!ws) {
        real_pa_stream_unref(rs);
        return NULL;
    }
    return (pa_stream *)ws;
}

pa_stream *pa_stream_new(pa_context *c, const char *name, const pa_sample_spec *ss, const pa_channel_map *map) {
    return pa_stream_new_with_proplist(c, name, ss, map, NULL);
}

pa_stream *pa_stream_new_extended(pa_context *c, const char *name, pa_format_info *const *formats, unsigned int n_formats, pa_proplist *p) {
    ensure_init();
    wctx_t *wctx = (wctx_t *)c;
    if (!is_valid_wctx(wctx)) return NULL;
    pa_stream *rs = NULL;
    if (!wctx->real_ctx) {
        if (!wctx->reconnecting || wctx->user_disconnected) return NULL;
    } else {
        rs = real_pa_stream_new_extended(wctx->real_ctx, name, formats, n_formats, p);
        if (!rs) return NULL;
    }
    pa_sample_spec init_ss = { .format = PA_SAMPLE_FLOAT32LE, .rate = 48000, .channels = 2 };
    pa_channel_map init_cmap;
    pa_channel_map_init_stereo(&init_cmap);
    if (n_formats > 0 && formats) {
        for (unsigned int i = 0; i < n_formats; i++) {
            if (!formats[i]) continue;
            pa_sample_spec cand_ss;
            pa_channel_map cand_cm;
            pa_sample_spec_init(&cand_ss);
            pa_channel_map_init(&cand_cm);
            if (pa_format_info_to_sample_spec(formats[i], &cand_ss, &cand_cm) == 0 &&
                pa_sample_spec_valid(&cand_ss)) {
                init_ss = cand_ss;
                if (pa_channel_map_valid(&cand_cm) && cand_cm.channels == cand_ss.channels) {
                    init_cmap = cand_cm;
                } else {
                    pa_channel_map_init_extend(&init_cmap, cand_ss.channels, PA_CHANNEL_MAP_DEFAULT);
                }
                break;
            }
        }
    }
    wstream_t *ws = alloc_wstream(wctx, rs, name, &init_ss, &init_cmap, p);
    if (!ws) {
        if (rs) real_pa_stream_unref(rs);
        return NULL;
    }
    if (!rs) ws->reconnecting = true;
    ws->created_extended = true;
    ws->n_formats = n_formats;
    if (n_formats > 0 && formats) {
        ws->formats = calloc(n_formats, sizeof(pa_format_info *));
        for (unsigned int i = 0; i < n_formats; i++) {
            if (formats[i]) ws->formats[i] = pa_format_info_copy(formats[i]);
        }
    }
    return (pa_stream *)ws;
}

pa_stream *pa_stream_ref(pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return s ? real_pa_stream_ref(s) : NULL;
    atomic_fetch_add(&ws->ref, 1);
    return s;
}

void pa_stream_unref(pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) { if (s) real_pa_stream_unref(s); return; }
    if (atomic_fetch_sub(&ws->ref, 1) > 1) return;

    pthread_mutex_lock(&g_stream_lock);
    wstream_t **gp = &g_stream_head;
    while (*gp) {
        if (*gp == ws) {
            *gp = ws->global_next;
            ws->global_next = NULL;
            break;
        }
        gp = &(*gp)->global_next;
    }
    pthread_mutex_unlock(&g_stream_lock);

    wctx_t *wctx = ws->wctx;
    pthread_mutex_lock(&wctx->lock);
    wstream_t **pp = &wctx->streams;
    while (*pp) {
        if (*pp == ws) {
            *pp = ws->next;
            break;
        }
        pp = &(*pp)->next;
    }
    pa_stream *rs = ws->real_stream;
    ws->real_stream = NULL;
    pthread_mutex_unlock(&wctx->lock);

    if (rs) {
        detach_stream_callbacks(rs);
        real_pa_stream_unref(rs);
    }

    free(ws->name);
    free(ws->dev);
    free(ws->dummy_write_buf);
    if (ws->proplist) pa_proplist_free(ws->proplist);
    if (ws->formats) {
        for (unsigned int i = 0; i < ws->n_formats; i++) {
            if (ws->formats[i]) pa_format_info_free(ws->formats[i]);
        }
        free(ws->formats);
    }
    free(ws);
    pa_context_unref((pa_context *)wctx);
}

pa_stream_state_t pa_stream_get_state(const pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return PA_STREAM_FAILED;
    if (ws->user_connected && !ws->user_disconnected && (ws->reconnecting || ws->wctx->reconnecting)) {
        return PA_STREAM_READY;
    }
    return ws->real_stream ? real_pa_stream_get_state(ws->real_stream) : ws->reported_state;
}

pa_context *pa_stream_get_context(const pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    return is_valid_wstream(ws) ? (pa_context *)ws->wctx : NULL;
}

uint32_t pa_stream_get_index(const pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return PA_INVALID_INDEX;
    if (ws->real_stream) {
        uint32_t idx = real_pa_stream_get_index(ws->real_stream);
        if (idx != PA_INVALID_INDEX) ws->last_stream_index = idx;
        return idx;
    }
    return ws->last_stream_index;
}

uint32_t pa_stream_get_device_index(const pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return PA_INVALID_INDEX;
    if (ws->real_stream) {
        uint32_t idx = real_pa_stream_get_device_index(ws->real_stream);
        if (idx != PA_INVALID_INDEX) ws->last_device_index = idx;
        return idx;
    }
    return ws->last_device_index;
}

const char *pa_stream_get_device_name(const pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return NULL;
    if (ws->real_stream) return real_pa_stream_get_device_name(ws->real_stream);
    return ws->dev ? ws->dev : "default";
}

int pa_stream_is_suspended(const pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return -1;
    return ws->real_stream ? real_pa_stream_is_suspended(ws->real_stream) : 0;
}

int pa_stream_is_corked(const pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return -1;
    return ws->real_stream ? real_pa_stream_is_corked(ws->real_stream) : ws->corked;
}

int pa_stream_connect_playback(pa_stream *s, const char *dev, const pa_buffer_attr *attr,
                               pa_stream_flags_t flags, const pa_cvolume *volume, pa_stream *sync_stream) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return -1;
    pthread_mutex_lock(&ws->wctx->lock);
    ws->is_record = false;
    ws->user_connected = true;
    ws->user_disconnected = false;
    free(ws->dev);
    ws->dev = xstrdup(dev);
    if (attr) { ws->attr = *attr; ws->has_attr = true; } else { ws->has_attr = false; }
    ws->flags = flags;
    ws->corked = (flags & PA_STREAM_START_CORKED) ? 1 : 0;
    if (volume) { ws->volume = *volume; ws->has_volume = true; } else { ws->has_volume = false; }
    ws->sync_stream = (wstream_t *)sync_stream;

    int rc = 0;
    if (ws->real_stream) {
        pa_stream *rs_sync = (ws->sync_stream && is_valid_wstream(ws->sync_stream)) ? ws->sync_stream->real_stream : NULL;
        rc = real_pa_stream_connect_playback(ws->real_stream, dev, attr, flags, volume, rs_sync);
    } else if (!ws->wctx->reconnecting) {
        rc = -1;
    } else {
        ws->reconnecting = true;
    }
    pthread_mutex_unlock(&ws->wctx->lock);
    return rc;
}

int pa_stream_connect_record(pa_stream *s, const char *dev, const pa_buffer_attr *attr, pa_stream_flags_t flags) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return -1;
    pthread_mutex_lock(&ws->wctx->lock);
    ws->is_record = true;
    ws->user_connected = true;
    ws->user_disconnected = false;
    free(ws->dev);
    ws->dev = xstrdup(dev);
    if (attr) { ws->attr = *attr; ws->has_attr = true; } else { ws->has_attr = false; }
    ws->flags = flags;
    ws->corked = (flags & PA_STREAM_START_CORKED) ? 1 : 0;

    int rc = 0;
    if (ws->real_stream) {
        rc = real_pa_stream_connect_record(ws->real_stream, dev, attr, flags);
    } else if (!ws->wctx->reconnecting) {
        rc = -1;
    } else {
        ws->reconnecting = true;
    }
    pthread_mutex_unlock(&ws->wctx->lock);
    return rc;
}

int pa_stream_connect_upload(pa_stream *s, size_t length) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws) || !ws->real_stream) return -1;
    return real_pa_stream_connect_upload(ws->real_stream, length);
}

int pa_stream_finish_upload(pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws) || !ws->real_stream) return -1;
    return real_pa_stream_finish_upload(ws->real_stream);
}

int pa_stream_disconnect(pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return -1;
    pthread_mutex_lock(&ws->wctx->lock);
    ws->user_connected = false;
    ws->user_disconnected = true;
    ws->reconnecting = false;
    ws->reported_state = PA_STREAM_TERMINATED;
    int rc = 0;
    if (ws->real_stream) {
        rc = real_pa_stream_disconnect(ws->real_stream);
    } else if (ws->state_cb) {
        pa_stream_notify_cb_t cb = ws->state_cb;
        void *ud = ws->state_ud;
        pthread_mutex_unlock(&ws->wctx->lock);
        cb(s, ud);
        return 0;
    }
    pthread_mutex_unlock(&ws->wctx->lock);
    return rc;
}

int pa_stream_begin_write(pa_stream *s, void **data, size_t *nbytes) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws) || !data || !nbytes) return -1;
    if (ws->real_stream) {
        ws->using_dummy_write = false;
        return real_pa_stream_begin_write(ws->real_stream, data, nbytes);
    }
    size_t req = (*nbytes == (size_t)-1 || *nbytes == 0) ? 4096 : *nbytes;
    if (req > ws->dummy_write_cap) {
        void *nb = realloc(ws->dummy_write_buf, req);
        if (!nb) return -1;
        ws->dummy_write_buf = nb;
        ws->dummy_write_cap = req;
    }
    ws->using_dummy_write = true;
    *data = ws->dummy_write_buf;
    *nbytes = req;
    return 0;
}

int pa_stream_cancel_write(pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return -1;
    if (ws->using_dummy_write || !ws->real_stream) {
        ws->using_dummy_write = false;
        return 0;
    }
    return real_pa_stream_cancel_write(ws->real_stream);
}

int pa_stream_write(pa_stream *s, const void *data, size_t nbytes, pa_free_cb_t free_cb, int64_t offset, pa_seek_mode_t seek) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return -1;
    if (!ws->real_stream || (ws->using_dummy_write && data == ws->dummy_write_buf)) {
        ws->using_dummy_write = false;
        if (free_cb && data != ws->dummy_write_buf) free_cb((void *)data);
        return 0;
    }
    return real_pa_stream_write(ws->real_stream, data, nbytes, free_cb, offset, seek);
}

int pa_stream_write_ext_free(pa_stream *s, const void *data, size_t nbytes, pa_free_cb_t free_cb, void *free_cb_data, int64_t offset, pa_seek_mode_t seek) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return s ? real_pa_stream_write_ext_free(s, data, nbytes, free_cb, free_cb_data, offset, seek) : -1;
    if (!ws->real_stream) {
        if (free_cb) free_cb(free_cb_data);
        return 0;
    }
    return real_pa_stream_write_ext_free(ws->real_stream, data, nbytes, free_cb, free_cb_data, offset, seek);
}

int pa_stream_peek(pa_stream *s, const void **data, size_t *nbytes) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws) || !data || !nbytes) return -1;
    if (!ws->real_stream) {
        *data = NULL;
        *nbytes = 0;
        return 0;
    }
    return real_pa_stream_peek(ws->real_stream, data, nbytes);
}

int pa_stream_drop(pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return -1;
    if (!ws->real_stream) return 0;
    return real_pa_stream_drop(ws->real_stream);
}

size_t pa_stream_writable_size(const pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return (size_t)-1;
    if (!ws->real_stream) return 0;
    return real_pa_stream_writable_size(ws->real_stream);
}

size_t pa_stream_readable_size(const pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return (size_t)-1;
    if (!ws->real_stream) return 0;
    return real_pa_stream_readable_size(ws->real_stream);
}

typedef struct {
    wstream_t *ws;
    pa_stream_success_cb_t cb;
    void *ud;
} stream_cb_wrap_t;

static void wrap_stream_success_cb(pa_stream *rs, int success, void *ud) {
    (void)rs;
    stream_cb_wrap_t *w = (stream_cb_wrap_t *)ud;
    if (is_valid_wstream(w->ws)) {
        pa_stream_ref((pa_stream *)w->ws);
        bool reconn = !w->ws->user_disconnected && w->ws->wctx && !w->ws->wctx->user_disconnected &&
                      (w->ws->reconnecting || w->ws->wctx->reconnecting || !w->ws->real_stream);
        int out = (!success && reconn) ? 1 : success;
        if (w->cb && !w->ws->user_disconnected) w->cb((pa_stream *)w->ws, out, w->ud);
        pa_stream_unref((pa_stream *)w->ws);
    }
    free(w);
}

static bool stream_can_synth_op(wstream_t *ws) {
    return ws && !ws->user_disconnected && ws->wctx && !ws->wctx->user_disconnected &&
           (ws->reconnecting || ws->wctx->reconnecting || !ws->real_stream);
}

static bool stream_needs_synth_now(wstream_t *ws) {
    if (!stream_can_synth_op(ws)) return false;
    if (!ws->real_stream) return true;
    return real_pa_stream_get_state(ws->real_stream) != PA_STREAM_READY;
}

static stream_cb_wrap_t *alloc_stream_wrap(wstream_t *ws, pa_stream_success_cb_t cb, void *userdata) {
    stream_cb_wrap_t *w = calloc(1, sizeof(stream_cb_wrap_t));
    if (w) { w->ws = ws; w->cb = cb; w->ud = userdata; }
    return w;
}

static pa_operation *finish_stream_op(wstream_t *ws, pa_operation *op, stream_cb_wrap_t *w,
                                      pa_stream_success_cb_t cb, void *userdata) {
    if (!op) {
        free(w);
        if (stream_can_synth_op(ws)) {
            return spawn_synth_stream_op(ws, (void *)cb, userdata);
        }
    }
    return op;
}

#define STREAM_OP_SUCCESS(fname, params, fwd_args) \
pa_operation *fname params { \
    ensure_init(); \
    wstream_t *ws = (wstream_t *)s; \
    if (!is_valid_wstream(ws)) return NULL; \
    if (stream_needs_synth_now(ws)) return spawn_synth_stream_op(ws, (void *)cb, userdata); \
    if (!ws->real_stream) return NULL; \
    stream_cb_wrap_t *w = alloc_stream_wrap(ws, cb, userdata); \
    pa_operation *op = real_##fname fwd_args; \
    return finish_stream_op(ws, op, w, cb, userdata); \
}

STREAM_OP_SUCCESS(pa_stream_drain, (pa_stream *s, pa_stream_success_cb_t cb, void *userdata), (ws->real_stream, w ? wrap_stream_success_cb : NULL, w))
pa_operation *pa_stream_update_timing_info(pa_stream *s, pa_stream_success_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return s ? real_pa_stream_update_timing_info(s, cb, userdata) : NULL;
    if (stream_needs_synth_now(ws)) return spawn_synth_stream_op(ws, (void *)cb, userdata);
    if (!ws->real_stream) return NULL;
    stream_cb_wrap_t *w = alloc_stream_wrap(ws, cb, userdata);
    pa_operation *op = real_pa_stream_update_timing_info(ws->real_stream, w ? wrap_stream_success_cb : NULL, w);
    return finish_stream_op(ws, op, w, cb, userdata);
}
STREAM_OP_SUCCESS(pa_stream_flush, (pa_stream *s, pa_stream_success_cb_t cb, void *userdata), (ws->real_stream, w ? wrap_stream_success_cb : NULL, w))
STREAM_OP_SUCCESS(pa_stream_prebuf, (pa_stream *s, pa_stream_success_cb_t cb, void *userdata), (ws->real_stream, w ? wrap_stream_success_cb : NULL, w))
STREAM_OP_SUCCESS(pa_stream_trigger, (pa_stream *s, pa_stream_success_cb_t cb, void *userdata), (ws->real_stream, w ? wrap_stream_success_cb : NULL, w))
pa_operation *pa_stream_proplist_update(pa_stream *s, pa_update_mode_t mode, pa_proplist *p, pa_stream_success_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return s ? real_pa_stream_proplist_update(s, mode, p, cb, userdata) : NULL;
    if (stream_needs_synth_now(ws)) return spawn_synth_stream_op(ws, (void *)cb, userdata);
    if (!ws->real_stream) return NULL;
    stream_cb_wrap_t *w = alloc_stream_wrap(ws, cb, userdata);
    pa_operation *op = real_pa_stream_proplist_update(ws->real_stream, mode, p, w ? wrap_stream_success_cb : NULL, w);
    return finish_stream_op(ws, op, w, cb, userdata);
}
STREAM_OP_SUCCESS(pa_stream_proplist_remove, (pa_stream *s, const char *const keys[], pa_stream_success_cb_t cb, void *userdata), (ws->real_stream, keys, w ? wrap_stream_success_cb : NULL, w))

pa_operation *pa_stream_cork(pa_stream *s, int b, pa_stream_success_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return NULL;
    ws->corked = b ? 1 : 0;
    if (stream_needs_synth_now(ws)) return spawn_synth_stream_op(ws, (void *)cb, userdata);
    if (!ws->real_stream) return NULL;
    stream_cb_wrap_t *w = alloc_stream_wrap(ws, cb, userdata);
    pa_operation *op = real_pa_stream_cork(ws->real_stream, b, w ? wrap_stream_success_cb : NULL, w);
    return finish_stream_op(ws, op, w, cb, userdata);
}

pa_operation *pa_stream_set_name(pa_stream *s, const char *name, pa_stream_success_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return NULL;
    free(ws->name);
    ws->name = xstrdup(name);
    if (stream_needs_synth_now(ws)) return spawn_synth_stream_op(ws, (void *)cb, userdata);
    if (!ws->real_stream) return NULL;
    stream_cb_wrap_t *w = alloc_stream_wrap(ws, cb, userdata);
    pa_operation *op = real_pa_stream_set_name(ws->real_stream, name, w ? wrap_stream_success_cb : NULL, w);
    return finish_stream_op(ws, op, w, cb, userdata);
}

pa_operation *pa_stream_set_buffer_attr(pa_stream *s, const pa_buffer_attr *attr, pa_stream_success_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return NULL;
    if (attr) { ws->attr = *attr; ws->has_attr = true; }
    if (stream_needs_synth_now(ws)) return spawn_synth_stream_op(ws, (void *)cb, userdata);
    if (!ws->real_stream) return NULL;
    stream_cb_wrap_t *w = alloc_stream_wrap(ws, cb, userdata);
    pa_operation *op = real_pa_stream_set_buffer_attr(ws->real_stream, attr, w ? wrap_stream_success_cb : NULL, w);
    return finish_stream_op(ws, op, w, cb, userdata);
}

pa_operation *pa_stream_update_sample_rate(pa_stream *s, uint32_t rate, pa_stream_success_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return NULL;
    ws->ss.rate = rate;
    if (stream_needs_synth_now(ws)) return spawn_synth_stream_op(ws, (void *)cb, userdata);
    if (!ws->real_stream) return NULL;
    stream_cb_wrap_t *w = alloc_stream_wrap(ws, cb, userdata);
    pa_operation *op = real_pa_stream_update_sample_rate(ws->real_stream, rate, w ? wrap_stream_success_cb : NULL, w);
    return finish_stream_op(ws, op, w, cb, userdata);
}

void pa_stream_set_state_callback(pa_stream *s, pa_stream_notify_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return;
    ws->state_cb = cb; ws->state_ud = userdata;
}

void pa_stream_set_write_callback(pa_stream *s, pa_stream_request_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return;
    ws->write_cb = cb; ws->write_ud = userdata;
    if (ws->real_stream) real_pa_stream_set_write_callback(ws->real_stream, cb ? internal_stream_write_cb : NULL, ws);
}

void pa_stream_set_read_callback(pa_stream *s, pa_stream_request_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return;
    ws->read_cb = cb; ws->read_ud = userdata;
    if (ws->real_stream) real_pa_stream_set_read_callback(ws->real_stream, cb ? internal_stream_read_cb : NULL, ws);
}

void pa_stream_set_overflow_callback(pa_stream *s, pa_stream_notify_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return;
    ws->overflow_cb = cb; ws->overflow_ud = userdata;
    if (ws->real_stream) real_pa_stream_set_overflow_callback(ws->real_stream, cb ? internal_stream_overflow_cb : NULL, ws);
}

void pa_stream_set_underflow_callback(pa_stream *s, pa_stream_notify_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return;
    ws->underflow_cb = cb; ws->underflow_ud = userdata;
    if (ws->real_stream) real_pa_stream_set_underflow_callback(ws->real_stream, cb ? internal_stream_underflow_cb : NULL, ws);
}

void pa_stream_set_started_callback(pa_stream *s, pa_stream_notify_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return;
    ws->started_cb = cb; ws->started_ud = userdata;
    if (ws->real_stream) real_pa_stream_set_started_callback(ws->real_stream, cb ? internal_stream_started_cb : NULL, ws);
}

void pa_stream_set_latency_update_callback(pa_stream *s, pa_stream_notify_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return;
    ws->latency_cb = cb; ws->latency_ud = userdata;
    if (ws->real_stream) real_pa_stream_set_latency_update_callback(ws->real_stream, cb ? internal_stream_latency_cb : NULL, ws);
}

void pa_stream_set_moved_callback(pa_stream *s, pa_stream_notify_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return;
    ws->moved_cb = cb; ws->moved_ud = userdata;
    if (ws->real_stream) real_pa_stream_set_moved_callback(ws->real_stream, cb ? internal_stream_moved_cb : NULL, ws);
}

void pa_stream_set_suspended_callback(pa_stream *s, pa_stream_notify_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return;
    ws->suspended_cb = cb; ws->suspended_ud = userdata;
    if (ws->real_stream) real_pa_stream_set_suspended_callback(ws->real_stream, cb ? internal_stream_suspended_cb : NULL, ws);
}

void pa_stream_set_event_callback(pa_stream *s, pa_stream_event_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return;
    ws->event_cb = cb; ws->event_ud = userdata;
    if (ws->real_stream) real_pa_stream_set_event_callback(ws->real_stream, cb ? internal_stream_event_cb : NULL, ws);
}

void pa_stream_set_buffer_attr_callback(pa_stream *s, pa_stream_notify_cb_t cb, void *userdata) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return;
    ws->buffer_attr_cb = cb; ws->buffer_attr_ud = userdata;
    if (ws->real_stream) real_pa_stream_set_buffer_attr_callback(ws->real_stream, cb ? internal_stream_buffer_attr_cb : NULL, ws);
}

int pa_stream_get_time(pa_stream *s, pa_usec_t *r_usec) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return s ? real_pa_stream_get_time(s, r_usec) : -1;

    pthread_mutex_lock(&ws->wctx->lock);
    if (!ws->real_stream) {
        if (r_usec) *r_usec = ws->last_reported_time_us;
        pthread_mutex_unlock(&ws->wctx->lock);
        return 0;
    }
    pa_usec_t raw = 0;
    int rc = real_pa_stream_get_time(ws->real_stream, &raw);
    if (rc == 0) {
        pa_usec_t total = ws->time_offset_us + raw;
        if (total < ws->last_reported_time_us) total = ws->last_reported_time_us;
        ws->last_reported_time_us = total;
        if (r_usec) *r_usec = total;
        pthread_mutex_unlock(&ws->wctx->lock);
        return 0;
    }
    if (ws->ever_ready || ws->reconnecting || ws->wctx->reconnecting) {
        if (r_usec) *r_usec = ws->last_reported_time_us;
        pthread_mutex_unlock(&ws->wctx->lock);
        return 0;
    }
    pthread_mutex_unlock(&ws->wctx->lock);
    return rc;
}

int pa_stream_get_latency(pa_stream *s, pa_usec_t *r_usec, int *negative) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return -1;
    if (!ws->real_stream) {
        if (r_usec) *r_usec = 20000;
        if (negative) *negative = 0;
        return 0;
    }
    int rc = real_pa_stream_get_latency(ws->real_stream, r_usec, negative);
    if (rc < 0 && (ws->ever_ready || ws->reconnecting || ws->wctx->reconnecting)) {
        if (r_usec) *r_usec = 20000;
        if (negative) *negative = 0;
        return 0;
    }
    return rc;
}

const pa_timing_info *pa_stream_get_timing_info(pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return NULL;
    if (ws->real_stream) {
        const pa_timing_info *ti = real_pa_stream_get_timing_info(ws->real_stream);
        if (ti) {
            ws->last_timing = *ti;
            ws->has_last_timing = true;
            return ti;
        }
        if (ws->ever_ready && ws->has_last_timing) {
            ws->last_timing.playing = !ws->corked;
            return &ws->last_timing;
        }
        return NULL;
    }
    ws->last_timing.playing = !ws->corked;
    return &ws->last_timing;
}

const pa_sample_spec *pa_stream_get_sample_spec(pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return NULL;
    return ws->real_stream ? real_pa_stream_get_sample_spec(ws->real_stream) : &ws->ss;
}

const pa_channel_map *pa_stream_get_channel_map(pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return NULL;
    return ws->real_stream ? real_pa_stream_get_channel_map(ws->real_stream) : &ws->cmap;
}

const pa_format_info *pa_stream_get_format_info(const pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return NULL;
    return ws->real_stream ? real_pa_stream_get_format_info(ws->real_stream) : (ws->formats && ws->n_formats > 0 ? ws->formats[0] : NULL);
}

const pa_buffer_attr *pa_stream_get_buffer_attr(pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return NULL;
    return ws->real_stream ? real_pa_stream_get_buffer_attr(ws->real_stream) : (ws->has_attr ? &ws->attr : NULL);
}

int pa_stream_set_monitor_stream(pa_stream *s, uint32_t sink_input_idx) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return -1;
    ws->has_monitor_stream = true;
    ws->monitor_stream = sink_input_idx;
    return ws->real_stream ? real_pa_stream_set_monitor_stream(ws->real_stream, sink_input_idx) : 0;
}

uint32_t pa_stream_get_monitor_stream(const pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return PA_INVALID_INDEX;
    return ws->real_stream ? real_pa_stream_get_monitor_stream(ws->real_stream) :
           (ws->has_monitor_stream ? ws->monitor_stream : PA_INVALID_INDEX);
}

int64_t pa_stream_get_underflow_index(const pa_stream *s) {
    ensure_init();
    wstream_t *ws = (wstream_t *)s;
    if (!is_valid_wstream(ws)) return -1;
    return ws->real_stream ? real_pa_stream_get_underflow_index(ws->real_stream) : 0;
}

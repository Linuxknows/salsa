// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![allow(unsafe_code)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::missing_safety_doc)]

use crate::c_abi::{
    copy_str_to_c, ptr_to_opt_str, LwasapiDbgPwClient, LwasapiDbgPwLink, LwasapiDbgPwNode,
    LwasapiDbgPwSnapshot, LwasapiPwBrokerOpts, LwasapiPwPreviewShm, LWASAPI_DBG_INFO,
    LWASAPI_DBG_PW_VIDEO, LWASAPI_DBG_WARN, LWASAPI_DEBUG_MAX_PW_CLIENTS,
    LWASAPI_DEBUG_MAX_PW_LINKS, LWASAPI_DEBUG_MAX_PW_NODES, LWASAPI_PW_PREVIEW_MAGIC,
    LWASAPI_PW_PREVIEW_MAX_H, LWASAPI_PW_PREVIEW_MAX_W, LWASAPI_PW_PREVIEW_VERSION,
};
use crate::dmabuf_gl::{DmabufFrameDesc, DmabufGlBlitter};
use crate::failsafe::{record_trip, FS_FD_QUOTA, FS_PW_BAD_PKT};
use crate::pw_wire::{
    build_negotiated_format_pod_iv, convert_preview_row, fd_quota_check, frame_pw_msg,
    intersect_vfmt, parse_param_pod, parse_pkt_hdr, parse_pkt_hdr_status, vfmt_name, vfmt_size,
    vfmt_stride, BufCaps, Dict, PktHdrStatus, PodBuilder, PodCursor, VfmtCaps, CMD_NODE,
    DATA_DMA_BUF, DATA_MEM_FD, DATA_MEM_ID, DRM_MOD_INVALID, EVT_NODE, IO_BUFFERS, IO_CLOCK,
    IO_POSITION, META_BUSY, META_HEADER, META_VIDEO_CROP, PARAM_ENUM_FORMAT, PARAM_FORMAT,
    PWB_MAX_PACKET_SZ, SPA_ASYNC_BIT, SPA_ID_INVALID, SPA_OBJECT, SPA_STRUCT,
};
use crate::unix_sys::{
    bind_unix_listener_0600, create_memfd, current_hostname, current_pid, current_uid,
    current_username, dbg_emit, dmabuf_sync, do_poll, dup_fd_cloexec, ensure_private_dir,
    eventfd_drain, eventfd_signal, memfd_size_valid, mono_ns, new_eventfd, open_private_shm,
    recvmsg_with_fds, seal_memfd_size, sendmsg_all_with_fds, set_thread_nice, socket_peercred,
    unix_socket_actively_served, unlink_shm_path, MappedMem, PollFd, RecvFdsResult, POLLERR,
    POLLHUP, POLLIN,
};

use core::ffi::{c_char, c_int};
use core::mem::{self, offset_of, size_of};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::fs;
use std::os::unix::io::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

const PWB_MAX_CLIENTS: usize = 64;
const PWB_MAX_CLIENT_OBJS: usize = 256;
const PWB_MAX_MEMS: usize = 1024;
const PWB_MAX_NODES: usize = 32;
const PWB_MAX_LINKS: usize = 32;
const PWB_MAX_BUFFERS: usize = 16;
const PWB_MAX_PARAMS: usize = 48;
const PWB_MAX_VFMT: usize = 32;

const PWB_ACT_SIZE: usize = 2312;
const PWB_ACT_POS_OFFSET: usize = 560;
const PWB_ACT_CLOCK_SIZE: usize = 160;
const PWB_ACT_POS_SIZE: usize = 1688;
const PWB_IO_BUFFERS_SIZE: usize = 16;
const PWB_META_STRIDE: usize = 4096;
const PWB_META_HDR_OFF: usize = 0;
const PWB_META_CROP_OFF: usize = 32;
const PWB_META_CHUNK_OFF: usize = 56;
const PWB_META_TOTAL_SZ: usize = 72;

const PWB_PERM_ALL: u32 = 0x1c8;
const PWB_STATUS_NEED_DATA: u32 = 1 << 0;
const PWB_STATUS_HAVE_DATA: u32 = 1 << 1;
const PWB_STATUS_OK: u32 = 0;
const PWB_DIR_IN: u8 = 0;
const PWB_DIR_OUT: u8 = 1;

const PW_CORE_M_HELLO: u8 = 1;
const PW_CORE_M_SYNC: u8 = 2;
const PW_CORE_M_GET_REGISTRY: u8 = 5;
const PW_CORE_M_CREATE_OBJ: u8 = 6;
const PW_CORE_M_DESTROY: u8 = 7;

const PW_CORE_E_INFO: u8 = 0;
const PW_CORE_E_DONE: u8 = 1;
const PW_CORE_E_ERROR: u8 = 3;
const PW_CORE_E_REMOVE_ID: u8 = 4;
const PW_CORE_E_ADD_MEM: u8 = 6;
const PW_CORE_E_REMOVE_MEM: u8 = 7;
const PW_CORE_E_BOUND_PROPS: u8 = 8;

const PW_REG_M_BIND: u8 = 1;
const PW_REG_M_DESTROY: u8 = 2;
const PW_REG_E_GLOBAL: u8 = 0;
const PW_REG_E_GLOBAL_REMOVE: u8 = 1;

const PW_CLIENT_M_UPDATE_PROPS: u8 = 2;
const PW_CLIENT_M_GET_PERMS: u8 = 3;
const PW_CLIENT_M_UPDATE_PERMS: u8 = 4;
const PW_CLIENT_E_INFO: u8 = 0;
const PW_CLIENT_E_PERMS: u8 = 1;

const PW_FACTORY_E_INFO: u8 = 0;
const PW_META_M_SET_PROPERTY: u8 = 1;
const PW_META_M_CLEAR: u8 = 2;
const PW_META_E_PROPERTY: u8 = 0;

const PW_NODE_M_ENUM_PARAMS: u8 = 2;
const PW_NODE_E_INFO: u8 = 0;
const PW_NODE_E_PARAM: u8 = 1;

const PW_PORT_M_ENUM_PARAMS: u8 = 2;
const PW_PORT_E_INFO: u8 = 0;
const PW_PORT_E_PARAM: u8 = 1;

const PW_LINK_E_INFO: u8 = 0;

const PW_CN_M_GET_NODE: u8 = 1;
const PW_CN_M_UPDATE: u8 = 2;
const PW_CN_M_PORT_UPDATE: u8 = 3;
const PW_CN_M_SET_ACTIVE: u8 = 4;
const PW_CN_M_EVENT: u8 = 5;
const PW_CN_M_PORT_BUFFERS: u8 = 6;

const PW_CN_E_TRANSPORT: u8 = 0;
const PW_CN_E_SET_IO: u8 = 2;
const PW_CN_E_COMMAND: u8 = 4;
const PW_CN_E_PORT_SET_PARAM: u8 = 7;
const PW_CN_E_PORT_USE_BUFS: u8 = 8;
const PW_CN_E_PORT_SET_IO: u8 = 9;
const PW_CN_E_SET_ACTIVATION: u8 = 10;
const PW_CN_E_PORT_SET_MIX_INFO: u8 = 11;

const SPA_NODE_CMD_PAUSE: u32 = 1;
const SPA_NODE_CMD_START: u32 = 2;
const SPA_NODE_EVT_REQUEST_PROCESS: u32 = 3;

const PROXY_KIND_REGISTRY: u8 = 1;
const PROXY_KIND_CLIENT: u8 = 2;
const PROXY_KIND_CLIENT_NODE: u8 = 3;
const PROXY_KIND_NODE: u8 = 4;
const PROXY_KIND_PORT: u8 = 5;
const PROXY_KIND_LINK: u8 = 6;
const PROXY_KIND_FACTORY: u8 = 7;
const PROXY_KIND_METADATA: u8 = 8;

#[inline]
fn atomic_u32_load(ptr: *const u32, ord: Ordering) -> u32 {
    unsafe { (&*ptr.cast::<AtomicU32>()).load(ord) }
}

#[inline]
fn atomic_u32_store(ptr: *mut u32, val: u32, ord: Ordering) {
    unsafe { (&*ptr.cast::<AtomicU32>()).store(val, ord) }
}

#[inline]
fn atomic_u64_load(ptr: *const u64, ord: Ordering) -> u64 {
    unsafe { (&*ptr.cast::<AtomicU64>()).load(ord) }
}

#[inline]
fn atomic_u64_store(ptr: *mut u64, val: u64, ord: Ordering) {
    unsafe { (&*ptr.cast::<AtomicU64>()).store(val, ord) }
}

fn pace_due(now: u64, next_due_ns: &mut u64, fps: u32) -> bool {
    if fps == 0 {
        *next_due_ns = 0;
        return true;
    }
    let period = 1_000_000_000u64 / (fps as u64);
    if period == 0 {
        return true;
    }
    if *next_due_ns == 0 || now >= *next_due_ns {
        if *next_due_ns == 0 || now - *next_due_ns > period.saturating_mul(2) {
            *next_due_ns = now + period;
        } else {
            *next_due_ns += period;
        }
        true
    } else {
        false
    }
}

fn update_fps_meter(now: u64, win_ns: &mut u64, win_cnt: &mut u32, out_x100: &mut u32) {
    if *win_ns == 0 {
        *win_ns = now;
        *win_cnt = 0;
        return;
    }
    *win_cnt = win_cnt.saturating_add(1);
    let el = now.saturating_sub(*win_ns);
    if el >= 500_000_000 {
        *out_x100 = (((*win_cnt as u128) * 100_000_000_000u128) / (el as u128)) as u32;
        *win_ns = now;
        *win_cnt = 0;
    }
}

fn parse_u32_strict(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let v: u32 = s.parse().ok()?;
    (v > 0).then_some(v)
}

fn detect_proc_info(pid: u32) -> (String, String) {
    let comm_path = format!("/proc/{}/comm", pid);
    let mut app = fs::read_to_string(comm_path)
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if app.is_empty() {
        app = format!("pid-{}", pid);
    }
    let exe_path = format!("/proc/{}/exe", pid);
    let bin = fs::read_link(exe_path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| app.clone());
    (app, bin)
}

#[derive(Clone, Copy, Default)]
struct PwProxy {
    in_use: bool,
    proxy_id: u32,
    global_id: u32,
    kind: u8,
}

struct PwClient {
    in_use: bool,
    disconnecting: bool,
    hello_done: bool,
    is_portal: bool,
    stream: Option<UnixStream>,
    global_id: u32,
    pid: u32,
    next_mem_id: u32,
    mem_free_head: u32,
    mem_free_next: [u32; PWB_MAX_MEMS],
    out_seq: u32,
    app_name: String,
    binary: String,
    portal_app_id: String,
    props: Dict,
    objs: [PwProxy; PWB_MAX_CLIENT_OBJS],
    rx_buf: Vec<u8>,
    rx_fds: Vec<OwnedFd>,
}

impl Default for PwClient {
    fn default() -> Self {
        Self {
            in_use: false,
            disconnecting: false,
            hello_done: false,
            is_portal: false,
            stream: None,
            global_id: 0,
            pid: 0,
            next_mem_id: 0,
            mem_free_head: SPA_ID_INVALID,
            mem_free_next: [SPA_ID_INVALID; PWB_MAX_MEMS],
            out_seq: 0,
            app_name: String::new(),
            binary: String::new(),
            portal_app_id: String::new(),
            props: Dict::default(),
            objs: [PwProxy::default(); PWB_MAX_CLIENT_OBJS],
            rx_buf: Vec::new(),
            rx_fds: Vec::new(),
        }
    }
}

impl PwClient {
    fn raw_fd(&self) -> RawFd {
        self.stream.as_ref().map_or(-1, AsRawFd::as_raw_fd)
    }

    fn send_msg(&mut self, obj_id: u32, op: u8, payload: &[u8], fds: &[RawFd]) -> bool {
        if !self.in_use || self.disconnecting {
            return false;
        }
        let fd = self.raw_fd();
        if fd < 0 {
            return false;
        }
        let seq = self.out_seq;
        self.out_seq = self.out_seq.wrapping_add(1);
        let pkt = frame_pw_msg(obj_id, op, seq, fds.len() as u32, payload);
        sendmsg_all_with_fds(fd, &pkt, fds).is_ok()
    }

    fn add_mem(&mut self, mem_type: u32, fd: RawFd, flags: u32) -> Option<u32> {
        if !self.in_use || self.disconnecting || fd < 0 {
            return None;
        }
        let from_free = self.mem_free_head != SPA_ID_INVALID;
        let mid = if from_free {
            self.mem_free_head
        } else {
            if (self.next_mem_id as usize) >= PWB_MAX_MEMS {
                return None;
            }
            self.next_mem_id
        };
        let mut b = PodBuilder::new(128);
        b.push_struct();
        b.int(mid as i32);
        b.id(mem_type);
        b.fd(0);
        b.int(flags as i32);
        b.pop();
        let pkt = b.into_vec();
        if !self.send_msg(0, PW_CORE_E_ADD_MEM, &pkt, &[fd]) {
            return None;
        }
        if from_free {
            self.mem_free_head = if (mid as usize) < PWB_MAX_MEMS {
                self.mem_free_next[mid as usize]
            } else {
                SPA_ID_INVALID
            };
        } else {
            self.next_mem_id += 1;
        }
        Some(mid)
    }

    fn remove_mem(&mut self, mem_id: &mut u32) {
        if *mem_id == SPA_ID_INVALID {
            return;
        }
        let mid = *mem_id;
        *mem_id = SPA_ID_INVALID;
        if !self.in_use || self.disconnecting || self.raw_fd() < 0 {
            return;
        }
        let mut b = PodBuilder::new(64);
        b.push_struct();
        b.int(mid as i32);
        b.pop();
        let pkt = b.into_vec();
        if self.send_msg(0, PW_CORE_E_REMOVE_MEM, &pkt, &[]) && (mid as usize) < PWB_MAX_MEMS {
            self.mem_free_next[mid as usize] = self.mem_free_head;
            self.mem_free_head = mid;
        }
    }

    fn send_remove_id(&mut self, obj_id: u32) {
        if !self.in_use || self.disconnecting || obj_id == 0 {
            return;
        }
        let mut b = PodBuilder::new(64);
        b.push_struct();
        b.int(obj_id as i32);
        b.pop();
        let pkt = b.into_vec();
        let _ = self.send_msg(0, PW_CORE_E_REMOVE_ID, &pkt, &[]);
    }

    fn alloc_proxy(&mut self, proxy_id: u32, global_id: u32, kind: u8) {
        if let Some(px) = self
            .objs
            .iter_mut()
            .find(|o| o.in_use && o.proxy_id == proxy_id)
        {
            px.global_id = global_id;
            px.kind = kind;
            return;
        }
        if let Some(px) = self.objs.iter_mut().find(|o| !o.in_use) {
            *px = PwProxy {
                in_use: true,
                proxy_id,
                global_id,
                kind,
            };
        }
    }

    fn find_proxy(&self, proxy_id: u32) -> Option<PwProxy> {
        self.objs
            .iter()
            .copied()
            .find(|o| o.in_use && o.proxy_id == proxy_id)
    }

    fn registry_proxy_ids(&self) -> Vec<u32> {
        self.objs
            .iter()
            .filter(|o| o.in_use && o.kind == PROXY_KIND_REGISTRY)
            .map(|o| o.proxy_id)
            .collect()
    }

    fn metadata_proxy_ids(&self, meta_gid: u32) -> Vec<u32> {
        self.objs
            .iter()
            .filter(|o| o.in_use && o.kind == PROXY_KIND_METADATA && o.global_id == meta_gid)
            .map(|o| o.proxy_id)
            .collect()
    }
}

#[derive(Clone)]
struct RawParam {
    id: u32,
    pod: Vec<u8>,
}

#[derive(Default)]
struct NodeBuf {
    fd: Option<OwnedFd>,
    mem_type: u32,
    _flags: u32,
    mapoffset: u32,
    maxsize: u32,
    mapped: Option<MappedMem>,
}

struct PwNode {
    in_use: bool,
    destroying: bool,
    is_driver: bool,
    autoconnect: bool,
    alloc_buffers: bool,
    port_registered: bool,
    mix_ready: bool,
    awaiting_port_buffers: bool,
    active: bool,
    neg_has_modifier: bool,
    neg_is_dmabuf: bool,
    direction: u8,
    state: u8,
    global_id: u32,
    port_global_id: u32,
    serial: u32,
    client_idx: u32,
    cn_proxy_id: u32,
    node_proxy_id: u32,
    pending_link_idx: u32,
    target_id_req: u32,
    target_str: String,
    name: String,
    media_class: String,
    node_props: Dict,
    port_props: Dict,
    node_params: Vec<RawParam>,
    port_params: Vec<RawParam>,
    vfmt_candidates: Vec<VfmtCaps>,
    bcaps: BufCaps,
    neg_format: u32,
    neg_w: u32,
    neg_h: u32,
    neg_stride: u32,
    neg_fps_num: u32,
    neg_fps_denom: u32,
    neg_modifier: u64,
    act_mem_fd: Option<OwnedFd>,
    act_map: Option<MappedMem>,
    act_readfd: Option<OwnedFd>,
    act_writefd: Option<OwnedFd>,
    act_mem_id: u32,
    done_efd: Option<OwnedFd>,
    done_act_fd: Option<OwnedFd>,
    done_act_map: Option<MappedMem>,
    done_act_memid: u32,
    meta_mem_fd: Option<OwnedFd>,
    meta_map: Option<MappedMem>,
    meta_mem_id: u32,
    io_mem_fd: Option<OwnedFd>,
    io_map: Option<MappedMem>,
    io_mem_id: u32,
    io_last_slot: u8,
    buf_slot: [u8; PWB_MAX_BUFFERS],
    n_bufs: u32,
    bufs: Vec<NodeBuf>,
    buf_mem_ids: [u32; PWB_MAX_BUFFERS],
    frames_count: u64,
    relay_next_due_ns: u64,
    relay_applied_fps: u32,
}

impl Default for PwNode {
    fn default() -> Self {
        Self {
            in_use: false,
            destroying: false,
            is_driver: false,
            autoconnect: false,
            alloc_buffers: false,
            port_registered: false,
            mix_ready: false,
            awaiting_port_buffers: false,
            active: false,
            neg_has_modifier: false,
            neg_is_dmabuf: false,
            direction: 0,
            state: 0,
            global_id: 0,
            port_global_id: 0,
            serial: 0,
            client_idx: 0,
            cn_proxy_id: 0,
            node_proxy_id: 0,
            pending_link_idx: 0,
            target_id_req: 0,
            target_str: String::new(),
            name: String::new(),
            media_class: String::new(),
            node_props: Dict::default(),
            port_props: Dict::default(),
            node_params: Vec::new(),
            port_params: Vec::new(),
            vfmt_candidates: Vec::new(),
            bcaps: BufCaps::default(),
            neg_format: 0,
            neg_w: 0,
            neg_h: 0,
            neg_stride: 0,
            neg_fps_num: 0,
            neg_fps_denom: 0,
            neg_modifier: DRM_MOD_INVALID,
            act_mem_fd: None,
            act_map: None,
            act_readfd: None,
            act_writefd: None,
            act_mem_id: SPA_ID_INVALID,
            done_efd: None,
            done_act_fd: None,
            done_act_map: None,
            done_act_memid: SPA_ID_INVALID,
            meta_mem_fd: None,
            meta_map: None,
            meta_mem_id: SPA_ID_INVALID,
            io_mem_fd: None,
            io_map: None,
            io_mem_id: SPA_ID_INVALID,
            io_last_slot: 0,
            buf_slot: [0; PWB_MAX_BUFFERS],
            n_bufs: 0,
            bufs: Vec::new(),
            buf_mem_ids: [SPA_ID_INVALID; PWB_MAX_BUFFERS],
            frames_count: 0,
            relay_next_due_ns: 0,
            relay_applied_fps: 0,
        }
    }
}

impl PwNode {
    fn free_buffers(&mut self) {
        self.bufs.clear();
        self.n_bufs = 0;
    }

    fn update_activation_pos(&mut self, driver_gid: u32) {
        let Some(map) = self.act_map.as_mut() else {
            return;
        };
        let slice = map.as_mut_slice();
        if slice.len() < PWB_ACT_SIZE {
            return;
        }
        let w = if self.neg_w != 0 { self.neg_w } else { 1920 };
        let h = if self.neg_h != 0 { self.neg_h } else { 1080 };
        let stride = if self.neg_stride != 0 {
            self.neg_stride
        } else {
            w * 4
        };
        let fn_v = if self.neg_fps_num != 0 {
            self.neg_fps_num
        } else {
            60
        };
        let fd_v = if self.neg_fps_denom != 0 {
            self.neg_fps_denom
        } else {
            1
        };

        slice[540..544].copy_from_slice(&1u32.to_le_bytes());
        slice[544..548].copy_from_slice(&1u32.to_le_bytes());
        slice[548..552].copy_from_slice(&driver_gid.to_le_bytes());
        slice[552..556].copy_from_slice(&driver_gid.to_le_bytes());
        slice[556..560].copy_from_slice(&1u32.to_le_bytes());

        let clk_id = if self.is_driver {
            self.global_id
        } else {
            driver_gid
        };
        slice[PWB_ACT_POS_OFFSET + 4..PWB_ACT_POS_OFFSET + 8]
            .copy_from_slice(&clk_id.to_le_bytes());
        let mut cname = [0u8; 64];
        copy_str_to_c(&mut cname, &format!("salsa-pw-{}", clk_id));
        slice[PWB_ACT_POS_OFFSET + 8..PWB_ACT_POS_OFFSET + 72].copy_from_slice(&cname);
        slice[PWB_ACT_POS_OFFSET + 80..PWB_ACT_POS_OFFSET + 84]
            .copy_from_slice(&1u32.to_le_bytes());
        slice[PWB_ACT_POS_OFFSET + 84..PWB_ACT_POS_OFFSET + 88]
            .copy_from_slice(&48000u32.to_le_bytes());
        slice[PWB_ACT_POS_OFFSET + 96..PWB_ACT_POS_OFFSET + 104]
            .copy_from_slice(&1600u64.to_le_bytes());
        slice[PWB_ACT_POS_OFFSET + 160..PWB_ACT_POS_OFFSET + 164]
            .copy_from_slice(&1u32.to_le_bytes());
        slice[PWB_ACT_POS_OFFSET + 164..PWB_ACT_POS_OFFSET + 168]
            .copy_from_slice(&stride.to_le_bytes());
        slice[PWB_ACT_POS_OFFSET + 168..PWB_ACT_POS_OFFSET + 172]
            .copy_from_slice(&w.to_le_bytes());
        slice[PWB_ACT_POS_OFFSET + 172..PWB_ACT_POS_OFFSET + 176]
            .copy_from_slice(&h.to_le_bytes());
        slice[PWB_ACT_POS_OFFSET + 176..PWB_ACT_POS_OFFSET + 180]
            .copy_from_slice(&fn_v.to_le_bytes());
        slice[PWB_ACT_POS_OFFSET + 180..PWB_ACT_POS_OFFSET + 184]
            .copy_from_slice(&fd_v.to_le_bytes());
        slice[PWB_ACT_POS_OFFSET + 208..PWB_ACT_POS_OFFSET + 212]
            .copy_from_slice(&2u32.to_le_bytes());
        slice[PWB_ACT_POS_OFFSET + 212..PWB_ACT_POS_OFFSET + 216]
            .copy_from_slice(&1u32.to_le_bytes());
        let now_ns = mono_ns();
        slice[PWB_ACT_POS_OFFSET + 72..PWB_ACT_POS_OFFSET + 80]
            .copy_from_slice(&now_ns.to_le_bytes());
    }

    fn stamp_activation_clock(&mut self, now_ns: u64) {
        let Some(map) = self.act_map.as_mut() else {
            return;
        };
        let slice = map.as_mut_slice();
        if slice.len() < PWB_ACT_POS_OFFSET + 96 {
            return;
        }
        let pos = ((now_ns as u128 * 48_000u128) / 1_000_000_000u128) as u64;
        slice[PWB_ACT_POS_OFFSET + 72..PWB_ACT_POS_OFFSET + 80]
            .copy_from_slice(&now_ns.to_le_bytes());
        slice[PWB_ACT_POS_OFFSET + 88..PWB_ACT_POS_OFFSET + 96]
            .copy_from_slice(&pos.to_le_bytes());
    }

    fn apply_target_props(&mut self) {
        if let Some(ac) = self.node_props.get_str("node.autoconnect") {
            if !ac.is_empty() {
                self.autoconnect = ac == "true";
            }
        }
        let tgt = self
            .node_props
            .get_str("target.object")
            .filter(|s| !s.is_empty())
            .or_else(|| {
                self.node_props
                    .get_str("node.target")
                    .filter(|s| !s.is_empty())
            });
        if let Some(t) = tgt {
            if t == "-1" {
                self.target_str.clear();
                self.target_id_req = 0;
            } else {
                self.target_id_req = parse_u32_strict(&t).unwrap_or(0);
                self.target_str = t;
            }
        }
    }

    fn build_public_props(&self) -> Dict {
        let mut d = self.node_props.clone();
        d.set_str("object.serial", &self.serial.to_string());
        d.set_str("object.id", &self.global_id.to_string());
        d.set_str("factory.id", "1");
        if d.get_str("node.name").is_none() {
            d.set_str("node.name", &self.name);
        }
        if d.get_str("media.class").is_none() {
            d.set_str("media.class", &self.media_class);
        }
        if d.get_str("media.type").is_none() {
            d.set_str("media.type", "Video");
        }
        if d.get_str("media.category").is_none() {
            d.set_str("media.category", "Capture");
        }
        d
    }

    fn io_read_slot(&self, slot: usize) -> Option<(u32, u32)> {
        let off = slot.checked_mul(8)?;
        let map = self.io_map.as_ref()?;
        let sl = map.as_slice();
        if off + 8 > sl.len() {
            return None;
        }
        let st = atomic_u32_load(unsafe { sl.as_ptr().add(off).cast::<u32>() }, Ordering::Acquire);
        let bid = u32::from_le_bytes([sl[off + 4], sl[off + 5], sl[off + 6], sl[off + 7]]);
        Some((st, bid))
    }

    fn io_read_status_and_buf(&self) -> Option<(u32, u32)> {
        self.io_read_slot(0)
    }

    fn io_write_slot(&mut self, slot: usize, status: u32, buf_id: u32) {
        let Some(off) = slot.checked_mul(8) else {
            return;
        };
        if let Some(map) = self.io_map.as_mut() {
            let sl = map.as_mut_slice();
            if off + 8 <= sl.len() {
                sl[off + 4..off + 8].copy_from_slice(&buf_id.to_le_bytes());
                atomic_u32_store(
                    unsafe { sl.as_mut_ptr().add(off).cast::<u32>() },
                    status,
                    Ordering::Release,
                );
            }
        }
    }

    fn io_write_slot_status(&mut self, slot: usize, status: u32) {
        let Some(off) = slot.checked_mul(8) else {
            return;
        };
        if let Some(map) = self.io_map.as_mut() {
            let sl = map.as_mut_slice();
            if off + 4 <= sl.len() {
                atomic_u32_store(
                    unsafe { sl.as_mut_ptr().add(off).cast::<u32>() },
                    status,
                    Ordering::Release,
                );
            }
        }
    }

    fn io_write_all_slots(&mut self, status: u32, buf_id: u32) {
        self.io_write_slot(0, status, buf_id);
        self.io_write_slot(1, status, buf_id);
    }

    fn io_take_ready_slot(&mut self) -> Option<(usize, u32)> {
        let s0 = self.io_read_slot(0);
        let s1 = self.io_read_slot(1);
        let h0 = s0.is_some_and(|(st, _)| st == PWB_STATUS_HAVE_DATA);
        let h1 = s1.is_some_and(|(st, _)| st == PWB_STATUS_HAVE_DATA);
        let chosen = match (h0, h1) {
            (false, false) => return None,
            (true, false) => 0,
            (false, true) => 1,
            (true, true) => {
                // If both async slots have HAVE_DATA after coalesced wakeups, the writer
                // wrote `io_last_slot ^ 1` first and wrapped to `io_last_slot` last.
                let newest = (self.io_last_slot as usize) & 1;
                let skipped = newest ^ 1;
                if let Some((_, skipped_bid)) = [s0, s1][skipped] {
                    self.io_write_slot(skipped, PWB_STATUS_NEED_DATA, skipped_bid);
                }
                newest
            }
        };
        let (_, buf_id) = [s0, s1][chosen].unwrap();
        self.io_last_slot = chosen as u8;
        if let Some(slot_ref) = self.buf_slot.get_mut(buf_id as usize) {
            *slot_ref = chosen as u8;
        }
        self.io_write_slot_status(chosen, PWB_STATUS_OK);
        Some((chosen, buf_id))
    }

    fn io_write_status_and_buf(&mut self, status: u32, buf_id: u32) {
        self.io_write_slot(0, status, buf_id);
    }

    fn io_write_status(&mut self, status: u32) {
        self.io_write_slot_status(0, status);
    }

    fn io_recycle_buf(&mut self, buf_id: u32) {
        let slot = self.buf_slot.get(buf_id as usize).copied().unwrap_or(0) as usize;
        if !self
            .io_read_slot(slot)
            .is_some_and(|(st, _)| st == PWB_STATUS_HAVE_DATA)
        {
            self.io_write_slot(slot, PWB_STATUS_NEED_DATA, buf_id);
        }
    }
}

#[derive(Default)]
struct PwLink {
    in_use: bool,
    awaiting_mod_fixation: bool,
    mod_fixated: bool,
    buffers_ready: bool,
    io_ready: bool,
    state: u8,
    global_id: u32,
    serial: u32,
    out_node_idx: u32,
    in_node_idx: u32,
    owner_client_idx: u32,
    _owner_proxy_id: u32,
    format_pod: Vec<u8>,
    frames_relayed: u64,
    last_buf_id: u32,
}

#[derive(Clone, Default)]
struct MetaEntry {
    subject: String,
    key: String,
    val_type: String,
    val: String,
}

struct SharedAtomics {
    running: AtomicBool,
    broker_listening: AtomicBool,
    zero_pw_mode: AtomicBool,
    preview_expiry_ns: AtomicU64,
    preview_target_w: AtomicU32,
    preview_target_h: AtomicU32,
    preview_target_fps: AtomicU32,
    preview_actual_fps_x100_pub: AtomicU32,
    cast_limit_w: AtomicU32,
    cast_limit_h: AtomicU32,
    cast_limit_fps: AtomicU32,
    frames_relayed_total: AtomicU64,
    frames_dropped: AtomicU64,
    audio_rejects: AtomicU32,
}

impl Default for SharedAtomics {
    fn default() -> Self {
        Self {
            running: AtomicBool::new(false),
            broker_listening: AtomicBool::new(false),
            zero_pw_mode: AtomicBool::new(false),
            preview_expiry_ns: AtomicU64::new(0),
            preview_target_w: AtomicU32::new(0),
            preview_target_h: AtomicU32::new(0),
            preview_target_fps: AtomicU32::new(0),
            preview_actual_fps_x100_pub: AtomicU32::new(0),
            cast_limit_w: AtomicU32::new(0),
            cast_limit_h: AtomicU32::new(0),
            cast_limit_fps: AtomicU32::new(0),
            frames_relayed_total: AtomicU64::new(0),
            frames_dropped: AtomicU64::new(0),
            audio_rejects: AtomicU32::new(0),
        }
    }
}

struct PwState {
    runtime_dir: String,
    socket_path: String,
    manager_socket_path: String,
    ignore_zero_flag: bool,
    listen: Option<UnixListener>,
    mgr: Option<UnixListener>,
    wake_efd: Option<OwnedFd>,
    next_global_id: u32,
    next_serial: u32,
    clients: Vec<PwClient>,
    nodes: Vec<PwNode>,
    links: Vec<PwLink>,
    metas: Vec<MetaEntry>,
    preview_shm_name: String,
    preview_shm_posix: bool,
    preview_shm_fd: Option<OwnedFd>,
    preview_shm_map: Option<MappedMem>,
    preview_next_due_ns: u64,
    preview_applied_fps: u32,
    preview_fps_win_ns: u64,
    preview_fps_win_cnt: u32,
    preview_actual_fps_x100: u32,
    relay_fps_win_ns: u64,
    relay_fps_win_cnt: u32,
    relay_actual_fps_x100: u32,
    relay_last_frame_ns: u64,
    dmabuf_blitter: Option<DmabufGlBlitter>,
    shared: Arc<SharedAtomics>,
}

impl PwState {
    fn new(shared: Arc<SharedAtomics>) -> Self {
        let mut clients = Vec::with_capacity(PWB_MAX_CLIENTS);
        for _ in 0..PWB_MAX_CLIENTS {
            clients.push(PwClient::default());
        }
        let mut nodes = Vec::with_capacity(PWB_MAX_NODES);
        for _ in 0..PWB_MAX_NODES {
            nodes.push(PwNode::default());
        }
        let mut links = Vec::with_capacity(PWB_MAX_LINKS);
        for _ in 0..PWB_MAX_LINKS {
            links.push(PwLink::default());
        }
        Self {
            runtime_dir: String::new(),
            socket_path: String::new(),
            manager_socket_path: String::new(),
            ignore_zero_flag: false,
            listen: None,
            mgr: None,
            wake_efd: None,
            next_global_id: 10,
            next_serial: 100,
            clients,
            nodes,
            links,
            metas: Vec::with_capacity(16),
            preview_shm_name: String::new(),
            preview_shm_posix: false,
            preview_shm_fd: None,
            preview_shm_map: None,
            preview_next_due_ns: 0,
            preview_applied_fps: 0,
            preview_fps_win_ns: 0,
            preview_fps_win_cnt: 0,
            preview_actual_fps_x100: 0,
            relay_fps_win_ns: 0,
            relay_fps_win_cnt: 0,
            relay_actual_fps_x100: 0,
            relay_last_frame_ns: 0,
            dmabuf_blitter: None,
            shared,
        }
    }

    fn zero_mode_configured(&self) -> bool {
        if self.ignore_zero_flag {
            return true;
        }
        if std::env::var("LWASAPI_PW_ZERO_MODE").as_deref() == Ok("1") {
            return true;
        }
        let Ok(home) = std::env::var("HOME") else {
            return false;
        };
        let path = format!("{}/.config/lwasapi/pipewire_zero_mode", home);
        let Ok(txt) = fs::read_to_string(path) else {
            return false;
        };
        let t = txt.trim();
        t.starts_with("enabled") || t.starts_with('1')
    }

    fn apply_bind_mode(&mut self) -> i32 {
        let want_zero = self.zero_mode_configured();
        self.shared.zero_pw_mode.store(want_zero, Ordering::Relaxed);
        if !want_zero {
            if self.listen.take().is_some() {
                let _ = fs::remove_file(&self.socket_path);
            }
            if self.mgr.take().is_some() {
                let _ = fs::remove_file(&self.manager_socket_path);
            }
            self.shared.broker_listening.store(false, Ordering::Relaxed);
            return 0;
        }
        if self.listen.is_some() {
            return 0;
        }
        if !self.ignore_zero_flag && unix_socket_actively_served(&self.socket_path, current_pid()) {
            dbg_emit(
                LWASAPI_DBG_PW_VIDEO,
                LWASAPI_DBG_WARN,
                current_pid(),
                0,
                "PW_BROKER_DEFER",
                &self.socket_path,
                &format!("upstream pipewire process still holding {}", self.socket_path),
            );
            return -16; // -EBUSY
        }
        self.listen = bind_unix_listener_0600(&self.socket_path);
        self.mgr = bind_unix_listener_0600(&self.manager_socket_path);
        let ok = self.listen.is_some();
        self.shared.broker_listening.store(ok, Ordering::Relaxed);
        if ok {
            dbg_emit(
                LWASAPI_DBG_PW_VIDEO,
                LWASAPI_DBG_INFO,
                current_pid(),
                0,
                "PW_BROKER_LISTEN",
                &self.socket_path,
                &format!(
                    "Built-in Rust Video-Only pipewire-0 ScreenCast broker ready (mgr={})",
                    self.manager_socket_path
                ),
            );
        }
        if ok {
            0
        } else {
            -1
        }
    }

    fn broadcast_global(&mut self, gid: u32, iface: &str, ver: u32, props: &Dict) {
        let mut b = PodBuilder::new(2048);
        b.push_struct();
        b.int(gid as i32);
        b.int(PWB_PERM_ALL as i32);
        b.str_val(Some(iface));
        b.int(ver as i32);
        b.dict(props);
        b.pop();
        let pay = b.into_vec();
        for c in &mut self.clients {
            if !c.in_use {
                continue;
            }
            for reg_id in c.registry_proxy_ids() {
                let _ = c.send_msg(reg_id, PW_REG_E_GLOBAL, &pay, &[]);
            }
        }
    }

    fn broadcast_global_remove(&mut self, gid: u32) {
        let mut b = PodBuilder::new(64);
        b.push_struct();
        b.int(gid as i32);
        b.pop();
        let pay = b.into_vec();
        for c in &mut self.clients {
            if !c.in_use {
                continue;
            }
            for reg_id in c.registry_proxy_ids() {
                let _ = c.send_msg(reg_id, PW_REG_E_GLOBAL_REMOVE, &pay, &[]);
            }
        }
    }

    fn emit_node_info_for_idx(&mut self, ci: usize, proxy_id: u32, nidx: usize) {
        let Some(nd) = self.nodes.get(nidx) else {
            return;
        };
        if !nd.in_use {
            return;
        }
        let props = nd.build_public_props();
        let mut pis: Vec<(u32, i32)> = Vec::new();
        if !nd.vfmt_candidates.is_empty() {
            pis.push((PARAM_ENUM_FORMAT, 2));
        }
        if nd.neg_w > 0 {
            pis.push((PARAM_FORMAT, 6));
        }
        let pw_st = if nd.state >= 1 {
            (nd.state - 1) as u32
        } else {
            1
        };
        let mut b = PodBuilder::new(2048);
        b.push_struct();
        b.int(nd.global_id as i32);
        b.int(i32::from(nd.direction == PWB_DIR_IN));
        b.int(i32::from(nd.direction == PWB_DIR_OUT));
        b.long(31);
        b.int(i32::from(nd.direction == PWB_DIR_IN && nd.port_registered));
        b.int(i32::from(nd.direction == PWB_DIR_OUT && nd.port_registered));
        b.id(pw_st);
        b.none();
        b.dict(&props);
        b.param_info(&pis);
        b.pop();
        let pay = b.into_vec();
        if let Some(c) = self.clients.get_mut(ci) {
            let _ = c.send_msg(proxy_id, PW_NODE_E_INFO, &pay, &[]);
        }
    }

    fn emit_port_info_for_idx(&mut self, ci: usize, proxy_id: u32, nidx: usize) {
        let Some(nd) = self.nodes.get(nidx) else {
            return;
        };
        if !nd.in_use {
            return;
        }
        let mut pis: Vec<(u32, i32)> = Vec::new();
        if !nd.port_params.is_empty() {
            pis.push((PARAM_ENUM_FORMAT, 2));
            if nd.neg_w > 0 {
                pis.push((PARAM_FORMAT, 6));
            }
        }
        let mut b = PodBuilder::new(2048);
        b.push_struct();
        b.int(nd.port_global_id as i32);
        b.int(nd.direction as i32);
        b.long(3);
        b.dict(&nd.port_props);
        b.param_info(&pis);
        b.pop();
        let pay = b.into_vec();
        if let Some(c) = self.clients.get_mut(ci) {
            let _ = c.send_msg(proxy_id, PW_PORT_E_INFO, &pay, &[]);
        }
    }

    fn emit_link_info_to(&mut self, ci: usize, proxy_id: u32, li: usize) {
        let Some(lk) = self.links.get(li) else {
            return;
        };
        if !lk.in_use {
            return;
        }
        let oi = lk.out_node_idx as usize;
        let ii = lk.in_node_idx as usize;
        let (on_gid, on_pgid) = (self.nodes[oi].global_id, self.nodes[oi].port_global_id);
        let (in_gid, in_pgid) = (self.nodes[ii].global_id, self.nodes[ii].port_global_id);

        let mut d = Dict::default();
        d.set_str("object.serial", &lk.serial.to_string());
        d.set_str("link.output.node", &on_gid.to_string());
        d.set_str("link.output.port", &on_pgid.to_string());
        d.set_str("link.input.node", &in_gid.to_string());
        d.set_str("link.input.port", &in_pgid.to_string());

        let mut b = PodBuilder::new(2048);
        b.push_struct();
        b.int(lk.global_id as i32);
        b.int(on_gid as i32);
        b.int(on_pgid as i32);
        b.int(in_gid as i32);
        b.int(in_pgid as i32);
        b.long(7);
        b.int(lk.state as i32);
        b.none();
        if lk.format_pod.len() >= 8 {
            b.raw_pod(&lk.format_pod);
        } else {
            b.none();
        }
        b.dict(&d);
        b.pop();
        let pay = b.into_vec();
        if let Some(c) = self.clients.get_mut(ci) {
            let _ = c.send_msg(proxy_id, PW_LINK_E_INFO, &pay, &[]);
        }
    }

    fn broadcast_node_updates(&mut self, nidx: usize) {
        let (nd_gid, pgid) = (self.nodes[nidx].global_id, self.nodes[nidx].port_global_id);
        for ci in 0..PWB_MAX_CLIENTS {
            if !self.clients[ci].in_use {
                continue;
            }
            let targets: Vec<(u8, u32)> = self.clients[ci]
                .objs
                .iter()
                .filter(|px| {
                    px.in_use
                        && ((px.kind == PROXY_KIND_NODE && px.global_id == nd_gid)
                            || (px.kind == PROXY_KIND_PORT && px.global_id == pgid))
                })
                .map(|px| (px.kind, px.proxy_id))
                .collect();
            for (kind, px_id) in targets {
                if kind == PROXY_KIND_NODE {
                    self.emit_node_info_for_idx(ci, px_id, nidx);
                } else {
                    self.emit_port_info_for_idx(ci, px_id, nidx);
                }
            }
        }
    }

    fn broadcast_metadata_prop(
        &mut self,
        meta_gid: u32,
        subj: u32,
        key: Option<&str>,
        val_type: Option<&str>,
        val: Option<&str>,
    ) {
        let mut b = PodBuilder::new(512);
        b.push_struct();
        b.int(subj as i32);
        match key.filter(|s| !s.is_empty()) {
            Some(k) => b.str_val(Some(k)),
            None => b.none(),
        }
        match val_type.filter(|s| !s.is_empty()) {
            Some(t) => b.str_val(Some(t)),
            None => b.none(),
        }
        match val.filter(|s| !s.is_empty()) {
            Some(v) => b.str_val(Some(v)),
            None => b.none(),
        }
        b.pop();
        let pay = b.into_vec();

        for c in &mut self.clients {
            if !c.in_use {
                continue;
            }
            let px_ids = c.metadata_proxy_ids(meta_gid);
            for pid in px_ids {
                let _ = c.send_msg(pid, PW_META_E_PROPERTY, &pay, &[]);
            }
        }
    }

    fn find_node_by_global(&self, gid: u32) -> Option<usize> {
        self.nodes
            .iter()
            .position(|nd| nd.in_use && nd.global_id == gid)
    }

    fn find_node_by_ref(&self, r: &str, want_dir: u8) -> Option<usize> {
        if r.is_empty() {
            return None;
        }
        let id_opt = parse_u32_strict(r);
        for (i, nd) in self.nodes.iter().enumerate() {
            if !nd.in_use {
                continue;
            }
            if let Some(id) = id_opt {
                if nd.global_id == id
                    || nd.serial == id
                    || (nd.port_registered && nd.port_global_id == id && nd.direction == want_dir)
                {
                    return Some(i);
                }
            }
            if nd.name == r && nd.direction == want_dir {
                return Some(i);
            }
        }
        None
    }

    fn node_is_linked(&self, nidx: usize, is_out: bool) -> bool {
        self.links.iter().any(|lk| {
            lk.in_use
                && ((is_out && lk.out_node_idx as usize == nidx)
                    || (!is_out && lk.in_node_idx as usize == nidx))
        })
    }

    fn update_preview_active_state(&self) {
        let any_active = self.links.iter().any(|lk| lk.in_use && lk.state == 4);
        if !any_active {
            if let Some(map) = self.preview_shm_map.as_ref() {
                let shm = map.as_ptr().cast::<LwasapiPwPreviewShm>();
                unsafe {
                    atomic_u32_store(ptr::addr_of!((*shm).active_stream).cast_mut(), 0, Ordering::Release);
                }
            }
        }
    }

    fn cn_send_port_set_param(
        &mut self,
        ci: usize,
        cn_proxy_id: u32,
        dir: u8,
        port_id: u32,
        param_id: u32,
        pod: &[u8],
    ) {
        let mut b = PodBuilder::new(2048);
        b.push_struct();
        b.int(dir as i32);
        b.int(port_id as i32);
        b.id(param_id);
        b.int(0);
        if pod.len() >= 8 {
            b.raw_pod(pod);
        } else {
            b.none();
        }
        b.pop();
        let pay = b.into_vec();
        if let Some(c) = self.clients.get_mut(ci) {
            let _ = c.send_msg(cn_proxy_id, PW_CN_E_PORT_SET_PARAM, &pay, &[]);
        }
    }

    fn cn_send_command(&mut self, ci: usize, cn_proxy_id: u32, cmd_id: u32) {
        let mut b = PodBuilder::new(128);
        b.push_struct();
        b.push_object(CMD_NODE, cmd_id);
        b.pop();
        b.pop();
        let pay = b.into_vec();
        if let Some(c) = self.clients.get_mut(ci) {
            let _ = c.send_msg(cn_proxy_id, PW_CN_E_COMMAND, &pay, &[]);
        }
    }

    fn cn_send_set_activation(
        &mut self,
        ci: usize,
        cn_proxy_id: u32,
        node_id: u32,
        signalfd: Option<RawFd>,
        memid: u32,
        off: u32,
        sz: u32,
    ) {
        let mut b = PodBuilder::new(160);
        b.push_struct();
        b.int(node_id as i32);
        b.fd(if signalfd.is_some() { 0 } else { -1 });
        b.int(memid as i32);
        b.int(off as i32);
        b.int(sz as i32);
        b.pop();
        let pay = b.into_vec();
        if let Some(c) = self.clients.get_mut(ci) {
            if let Some(sfd) = signalfd {
                let _ = c.send_msg(cn_proxy_id, PW_CN_E_SET_ACTIVATION, &pay, &[sfd]);
            } else {
                let _ = c.send_msg(cn_proxy_id, PW_CN_E_SET_ACTIVATION, &pay, &[]);
            }
        }
    }

    fn cn_send_port_set_io(
        &mut self,
        ci: usize,
        cn_proxy_id: u32,
        dir: u8,
        port_id: u32,
        mix_id: u32,
        io_id: u32,
        memid: u32,
        off: u32,
        sz: u32,
    ) {
        let mut b = PodBuilder::new(160);
        b.push_struct();
        b.int(dir as i32);
        b.int(port_id as i32);
        b.int(mix_id as i32);
        b.id(io_id);
        b.int(memid as i32);
        b.int(off as i32);
        b.int(sz as i32);
        b.pop();
        let pay = b.into_vec();
        if let Some(c) = self.clients.get_mut(ci) {
            let _ = c.send_msg(cn_proxy_id, PW_CN_E_PORT_SET_IO, &pay, &[]);
        }
    }

    fn cn_send_port_use_buffers_clear(&mut self, ci: usize, nidx: usize, mix_id: u32) {
        let (dir, cn_proxy_id) = (self.nodes[nidx].direction, self.nodes[nidx].cn_proxy_id);
        let mut b = PodBuilder::new(128);
        b.push_struct();
        b.int(dir as i32);
        b.int(0);
        b.int(mix_id as i32);
        b.int(0);
        b.int(0);
        b.pop();
        let pay = b.into_vec();
        if let Some(c) = self.clients.get_mut(ci) {
            let _ = c.send_msg(cn_proxy_id, PW_CN_E_PORT_USE_BUFS, &pay, &[]);
        }
    }

    fn cn_send_port_set_mix_info(
        &mut self,
        ci: usize,
        cn_proxy_id: u32,
        dir: u8,
        port_id: u32,
        mix_id: u32,
        peer_id: u32,
    ) {
        let mut b = PodBuilder::new(256);
        b.push_struct();
        b.int(dir as i32);
        b.int(port_id as i32);
        b.int(mix_id as i32);
        b.int(peer_id as i32);
        let empty = Dict::default();
        b.dict(&empty);
        b.pop();
        let pay = b.into_vec();
        if let Some(c) = self.clients.get_mut(ci) {
            let _ = c.send_msg(cn_proxy_id, PW_CN_E_PORT_SET_MIX_INFO, &pay, &[]);
        }
    }

    fn cn_send_port_use_buffers(
        &mut self,
        ci: usize,
        nidx: usize,
        mix_id: u32,
        flags: u32,
        meta_mem_id: u32,
        items: &[(u32, u32, u32, u32, u32)],
    ) {
        let (dir, cn_proxy_id) = (self.nodes[nidx].direction, self.nodes[nidx].cn_proxy_id);
        let mut b = PodBuilder::new(4096);
        b.push_struct();
        b.int(dir as i32);
        b.int(0);
        b.int(mix_id as i32);
        b.int(flags as i32);
        b.int(items.len() as i32);
        for (i, &(dt, ptr_id, dflags, moff, msz)) in items.iter().enumerate() {
            b.int(meta_mem_id as i32);
            b.int((i * PWB_META_STRIDE) as i32);
            b.int(PWB_META_TOTAL_SZ as i32);
            b.int(3);
            b.id(META_HEADER);
            b.int(32);
            b.id(META_VIDEO_CROP);
            b.int(16);
            b.id(META_BUSY);
            b.int(8);
            b.int(1);
            b.id(dt);
            b.int(ptr_id as i32);
            b.int(dflags as i32);
            b.int(moff as i32);
            b.int(msz as i32);
        }
        b.pop();
        let pay = b.into_vec();
        if let Some(c) = self.clients.get_mut(ci) {
            let _ = c.send_msg(cn_proxy_id, PW_CN_E_PORT_USE_BUFS, &pay, &[]);
        }
    }

    fn unwire_node_link_side(&mut self, nidx: usize, peer_gid: u32, is_out: bool) {
        if nidx >= PWB_MAX_NODES || !self.nodes[nidx].in_use {
            return;
        }
        let cidx = self.nodes[nidx].client_idx as usize;
        let can_send = self
            .clients
            .get(cidx)
            .is_some_and(|c| c.in_use && !c.disconnecting && c.raw_fd() >= 0);
        let linked_after = self.node_is_linked(nidx, is_out);
        let cn_proxy_id = self.nodes[nidx].cn_proxy_id;
        let destroying = self.nodes[nidx].destroying;
        let state = self.nodes[nidx].state;

        if can_send && !destroying && cn_proxy_id != 0 {
            if peer_gid != 0 {
                self.cn_send_set_activation(cidx, cn_proxy_id, peer_gid, None, SPA_ID_INVALID, 0, 0);
            }
            if !linked_after {
                if state == 4 {
                    self.cn_send_command(cidx, cn_proxy_id, SPA_NODE_CMD_PAUSE);
                }
                self.cn_send_port_use_buffers_clear(cidx, nidx, 0);
                self.cn_send_port_set_io(
                    cidx,
                    cn_proxy_id,
                    if is_out { PWB_DIR_OUT } else { PWB_DIR_IN },
                    0,
                    0,
                    IO_BUFFERS,
                    SPA_ID_INVALID,
                    0,
                    0,
                );
            }
        }

        if !linked_after {
            let mut meta_id = self.nodes[nidx].meta_mem_id;
            let mut io_id = self.nodes[nidx].io_mem_id;
            let mut done_act_id = self.nodes[nidx].done_act_memid;
            let mut buf_ids = self.nodes[nidx].buf_mem_ids;
            if can_send {
                if let Some(c) = self.clients.get_mut(cidx) {
                    c.remove_mem(&mut meta_id);
                    c.remove_mem(&mut io_id);
                    c.remove_mem(&mut done_act_id);
                    for bid in &mut buf_ids {
                        c.remove_mem(bid);
                    }
                }
            }
            let nd = &mut self.nodes[nidx];
            nd.meta_mem_id = meta_id;
            nd.io_mem_id = io_id;
            nd.done_act_memid = done_act_id;
            nd.buf_mem_ids = buf_ids;
            nd.meta_map = None;
            nd.meta_mem_fd = None;
            nd.io_map = None;
            nd.io_mem_fd = None;
            nd.io_last_slot = 0;
            nd.buf_slot = [0; PWB_MAX_BUFFERS];
            if is_out {
                nd.free_buffers();
            }
            nd.mix_ready = false;
            nd.awaiting_port_buffers = false;
            nd.state = if nd.active { 3 } else { 2 };
        }
    }

    fn destroy_link(&mut self, li: usize) {
        if li >= PWB_MAX_LINKS || !self.links[li].in_use {
            return;
        }
        let lgid = self.links[li].global_id;
        let out_idx = self.links[li].out_node_idx as usize;
        let in_idx = self.links[li].in_node_idx as usize;
        let prod_gid = self
            .nodes
            .get(out_idx)
            .filter(|n| n.in_use)
            .map_or(0, |n| n.global_id);
        let cons_gid = self
            .nodes
            .get(in_idx)
            .filter(|n| n.in_use)
            .map_or(0, |n| n.global_id);

        self.links[li].in_use = false;
        self.broadcast_global_remove(lgid);
        self.unwire_node_link_side(in_idx, prod_gid, false);
        self.unwire_node_link_side(out_idx, cons_gid, true);
        self.update_preview_active_state();
    }

    fn destroy_node(&mut self, nidx: usize) {
        if nidx >= PWB_MAX_NODES || !self.nodes[nidx].in_use {
            return;
        }
        self.nodes[nidx].destroying = true;
        for li in 0..PWB_MAX_LINKS {
            if self.links[li].in_use
                && (self.links[li].out_node_idx as usize == nidx
                    || self.links[li].in_node_idx as usize == nidx)
            {
                self.destroy_link(li);
            }
        }

        let owner_idx = self.nodes[nidx].client_idx as usize;
        let nd_gid = self.nodes[nidx].global_id;
        let port_reg = self.nodes[nidx].port_registered;
        let port_gid = self.nodes[nidx].port_global_id;

        if self
            .clients
            .get(owner_idx)
            .is_some_and(|c| c.in_use && !c.disconnecting && c.raw_fd() >= 0)
        {
            let mut ids = [
                self.nodes[nidx].act_mem_id,
                self.nodes[nidx].done_act_memid,
                self.nodes[nidx].meta_mem_id,
                self.nodes[nidx].io_mem_id,
            ];
            let mut bids = self.nodes[nidx].buf_mem_ids;
            if let Some(c) = self.clients.get_mut(owner_idx) {
                for id in &mut ids {
                    c.remove_mem(id);
                }
                for id in &mut bids {
                    c.remove_mem(id);
                }
            }
        }

        if port_reg {
            self.broadcast_global_remove(port_gid);
        }
        self.broadcast_global_remove(nd_gid);

        self.nodes[nidx] = PwNode::default();
        self.update_preview_active_state();
    }

    fn handle_client_disconnect(&mut self, cidx: usize) {
        if cidx >= PWB_MAX_CLIENTS || !self.clients[cidx].in_use {
            return;
        }
        self.clients[cidx].disconnecting = true;
        let cgid = self.clients[cidx].global_id;

        for i in 0..PWB_MAX_NODES {
            if self.nodes[i].in_use && self.nodes[i].client_idx as usize == cidx {
                self.destroy_node(i);
            }
        }
        for i in 0..PWB_MAX_LINKS {
            if self.links[i].in_use && self.links[i].owner_client_idx as usize == cidx {
                self.destroy_link(i);
            }
        }
        self.broadcast_global_remove(cgid);
        self.clients[cidx] = PwClient::default();
    }

    fn complete_link_with_prod_buffers(&mut self, li: usize) {
        if li >= PWB_MAX_LINKS || !self.links[li].in_use {
            return;
        }
        let prod_idx = self.links[li].out_node_idx as usize;
        let cons_idx = self.links[li].in_node_idx as usize;
        let pc_idx = self.nodes[prod_idx].client_idx as usize;
        let cc_idx = self.nodes[cons_idx].client_idx as usize;
        if !self.clients.get(pc_idx).is_some_and(|c| c.in_use)
            || !self.clients.get(cc_idx).is_some_and(|c| c.in_use)
            || !self.nodes[prod_idx].in_use
            || !self.nodes[cons_idx].in_use
            || self.nodes[prod_idx].n_bufs == 0
        {
            return;
        }
        let n = (self.nodes[prod_idx].n_bufs as usize).min(PWB_MAX_BUFFERS);
        let msz = n * PWB_META_STRIDE;

        let mut old_meta_id = self.nodes[cons_idx].meta_mem_id;
        let mut old_buf_ids = self.nodes[cons_idx].buf_mem_ids;
        self.clients[cc_idx].remove_mem(&mut old_meta_id);
        for bid in &mut old_buf_ids {
            self.clients[cc_idx].remove_mem(bid);
        }
        self.nodes[cons_idx].meta_mem_id = old_meta_id;
        self.nodes[cons_idx].buf_mem_ids = old_buf_ids;
        self.nodes[cons_idx].meta_map = None;
        self.nodes[cons_idx].meta_mem_fd = None;

        let Some(cmeta_fd) = create_memfd("salsa-pw-cmeta", msz) else {
            return;
        };
        let cmeta_map = MappedMem::map_fd(cmeta_fd.as_raw_fd(), msz, true, 0);
        seal_memfd_size(cmeta_fd.as_raw_fd());
        let Some(cmeta_mid) =
            self.clients[cc_idx].add_mem(DATA_MEM_FD, cmeta_fd.as_raw_fd(), 3)
        else {
            return;
        };
        self.nodes[cons_idx].meta_mem_fd = Some(cmeta_fd);
        self.nodes[cons_idx].meta_map = cmeta_map;
        self.nodes[cons_idx].meta_mem_id = cmeta_mid;

        let mut buf_items = Vec::with_capacity(n);
        for i in 0..n {
            let (raw_fd, mtype, moff, msz_b) = {
                let pb = &self.nodes[prod_idx].bufs[i];
                let Some(f) = pb.fd.as_ref() else {
                    return;
                };
                (
                    f.as_raw_fd(),
                    if pb.mem_type != 0 {
                        pb.mem_type
                    } else {
                        DATA_MEM_FD
                    },
                    pb.mapoffset,
                    pb.maxsize,
                )
            };
            let Some(dup) = dup_fd_cloexec(raw_fd) else {
                return;
            };
            let Some(cmid) = self.clients[cc_idx].add_mem(mtype, dup.as_raw_fd(), 3) else {
                return;
            };
            drop(dup);
            self.nodes[cons_idx].buf_mem_ids[i] = cmid;
            let dflag = if mtype == DATA_DMA_BUF { 3u32 } else { 11u32 };
            buf_items.push((DATA_MEM_ID, cmid, dflag, moff, msz_b));
        }

        let prod_gid = self.nodes[prod_idx].global_id;
        let cons_gid = self.nodes[cons_idx].global_id;
        let prod_cn = self.nodes[prod_idx].cn_proxy_id;
        let cons_cn = self.nodes[cons_idx].cn_proxy_id;

        if !self.nodes[prod_idx].mix_ready {
            self.cn_send_port_set_mix_info(pc_idx, prod_cn, PWB_DIR_OUT, 0, 0, cons_gid);
            self.nodes[prod_idx].mix_ready = true;
        }
        if !self.nodes[cons_idx].mix_ready {
            self.cn_send_port_set_mix_info(cc_idx, cons_cn, PWB_DIR_IN, 0, 0, prod_gid);
            self.nodes[cons_idx].mix_ready = true;
        }

        self.cn_send_port_use_buffers(cc_idx, cons_idx, 0, 0, cmeta_mid, &buf_items);

        if self.nodes[prod_idx].io_mem_fd.is_none() {
            if let Some(pfd) = create_memfd("salsa-pw-pio", PWB_IO_BUFFERS_SIZE) {
                let pmap = MappedMem::map_fd(pfd.as_raw_fd(), PWB_IO_BUFFERS_SIZE, true, 0);
                seal_memfd_size(pfd.as_raw_fd());
                let mid = self.clients[pc_idx]
                    .add_mem(DATA_MEM_FD, pfd.as_raw_fd(), 3)
                    .unwrap_or(SPA_ID_INVALID);
                self.nodes[prod_idx].io_mem_fd = Some(pfd);
                self.nodes[prod_idx].io_map = pmap;
                self.nodes[prod_idx].io_mem_id = mid;
                self.nodes[prod_idx].io_last_slot = 0;
                self.nodes[prod_idx].buf_slot = [0; PWB_MAX_BUFFERS];
                self.nodes[prod_idx].io_write_all_slots(PWB_STATUS_NEED_DATA, SPA_ID_INVALID);
            }
        }
        if self.nodes[cons_idx].io_mem_fd.is_none() {
            if let Some(cfd) = create_memfd("salsa-pw-cio", PWB_IO_BUFFERS_SIZE) {
                let cmap = MappedMem::map_fd(cfd.as_raw_fd(), PWB_IO_BUFFERS_SIZE, true, 0);
                seal_memfd_size(cfd.as_raw_fd());
                let mid = self.clients[cc_idx]
                    .add_mem(DATA_MEM_FD, cfd.as_raw_fd(), 3)
                    .unwrap_or(SPA_ID_INVALID);
                self.nodes[cons_idx].io_mem_fd = Some(cfd);
                self.nodes[cons_idx].io_map = cmap;
                self.nodes[cons_idx].io_mem_id = mid;
                self.nodes[cons_idx].io_last_slot = 0;
                self.nodes[cons_idx].io_write_all_slots(PWB_STATUS_OK, SPA_ID_INVALID);
            }
        }

        for idx in [prod_idx, cons_idx] {
            if let Some(dmap) = self.nodes[idx].done_act_map.as_mut() {
                let sl = dmap.as_mut_slice();
                if sl.len() >= PWB_ACT_SIZE {
                    sl[540..544].copy_from_slice(&1u32.to_le_bytes());
                    sl[544..548].copy_from_slice(&1u32.to_le_bytes());
                    sl[548..552].copy_from_slice(&prod_gid.to_le_bytes());
                    sl[552..556].copy_from_slice(&prod_gid.to_le_bytes());
                    sl[556..560].copy_from_slice(&1u32.to_le_bytes());
                    sl[0..4].copy_from_slice(&0u32.to_le_bytes());
                    sl[8..12].copy_from_slice(&0i32.to_le_bytes());
                    sl[12..16].copy_from_slice(&1i32.to_le_bytes());
                    sl[16..20].copy_from_slice(&1i32.to_le_bytes());
                }
            }
            self.nodes[idx].update_activation_pos(prod_gid);
        }

        if let (Some(daf), Some(defd)) = (
            self.nodes[prod_idx].done_act_fd.as_ref().map(AsRawFd::as_raw_fd),
            self.nodes[prod_idx].done_efd.as_ref().map(AsRawFd::as_raw_fd),
        ) {
            let mut old = self.nodes[prod_idx].done_act_memid;
            self.clients[pc_idx].remove_mem(&mut old);
            if let Some(mid) = self.clients[pc_idx].add_mem(DATA_MEM_FD, daf, 3) {
                self.nodes[prod_idx].done_act_memid = mid;
                self.cn_send_set_activation(
                    pc_idx,
                    prod_cn,
                    cons_gid,
                    Some(defd),
                    mid,
                    0,
                    PWB_ACT_SIZE as u32,
                );
            }
        }
        if let (Some(daf), Some(defd)) = (
            self.nodes[cons_idx].done_act_fd.as_ref().map(AsRawFd::as_raw_fd),
            self.nodes[cons_idx].done_efd.as_ref().map(AsRawFd::as_raw_fd),
        ) {
            let mut old = self.nodes[cons_idx].done_act_memid;
            self.clients[cc_idx].remove_mem(&mut old);
            if let Some(mid) = self.clients[cc_idx].add_mem(DATA_MEM_FD, daf, 3) {
                self.nodes[cons_idx].done_act_memid = mid;
                self.cn_send_set_activation(
                    cc_idx,
                    cons_cn,
                    prod_gid,
                    Some(defd),
                    mid,
                    0,
                    PWB_ACT_SIZE as u32,
                );
            }
        }

        let cons_ioid = self.nodes[cons_idx].io_mem_id;
        let prod_ioid = self.nodes[prod_idx].io_mem_id;
        self.cn_send_port_set_io(cc_idx, cons_cn, PWB_DIR_IN, 0, 0, IO_BUFFERS, cons_ioid, 0, 16);
        self.cn_send_port_set_io(pc_idx, prod_cn, PWB_DIR_OUT, 0, 0, IO_BUFFERS, prod_ioid, 0, 16);

        self.nodes[cons_idx].state = 4;
        self.nodes[prod_idx].state = 4;
        self.links[li].state = 4;
        self.links[li].buffers_ready = true;
        self.links[li].io_ready = true;

        self.cn_send_command(cc_idx, cons_cn, SPA_NODE_CMD_START);
        self.cn_send_command(pc_idx, prod_cn, SPA_NODE_CMD_START);

        self.broadcast_node_updates(prod_idx);
        self.broadcast_node_updates(cons_idx);

        let mut ld = Dict::default();
        ld.set_str("link.output.node", &prod_gid.to_string());
        ld.set_str("link.input.node", &cons_gid.to_string());
        let lgid = self.links[li].global_id;
        self.broadcast_global(lgid, "PipeWire:Interface:Link", 3, &ld);

        let prod_pid = self.clients[pc_idx].pid;
        let det = format!(
            "prod={}({}) -> cons={}({}) {}x{}@{}/{} {} {} nbuf={}",
            self.nodes[prod_idx].name,
            prod_gid,
            self.nodes[cons_idx].name,
            cons_gid,
            self.nodes[prod_idx].neg_w,
            self.nodes[prod_idx].neg_h,
            self.nodes[prod_idx].neg_fps_num,
            self.nodes[prod_idx].neg_fps_denom,
            vfmt_name(self.nodes[prod_idx].neg_format),
            if self.nodes[prod_idx].neg_is_dmabuf {
                "DmaBuf"
            } else {
                "MemFd"
            },
            n
        );
        dbg_emit(
            LWASAPI_DBG_PW_VIDEO,
            LWASAPI_DBG_INFO,
            prod_pid,
            prod_gid,
            "PW_LINK_ACTIVE",
            &self.nodes[cons_idx].name,
            &det,
        );
    }

    fn negotiate_and_activate_link(&mut self, li: usize) -> bool {
        let prod_idx = self.links[li].out_node_idx as usize;
        let cons_idx = self.links[li].in_node_idx as usize;
        let pc_idx = self.nodes[prod_idx].client_idx as usize;
        let cc_idx = self.nodes[cons_idx].client_idx as usize;
        if !self.clients.get(pc_idx).is_some_and(|c| c.in_use)
            || !self.clients.get(cc_idx).is_some_and(|c| c.in_use)
            || self.nodes[prod_idx].vfmt_candidates.is_empty()
        {
            return false;
        }

        let lim_w = self.shared.cast_limit_w.load(Ordering::Relaxed);
        let lim_h = self.shared.cast_limit_h.load(Ordering::Relaxed);
        let lim_fps = self.shared.cast_limit_fps.load(Ordering::Relaxed);
        let c_allows_dmabuf = !self.nodes[cons_idx].bcaps.valid
            || (self.nodes[cons_idx].bcaps.datatype_mask & (1 << DATA_DMA_BUF)) != 0;
        let p_allows_dmabuf = !self.nodes[prod_idx].bcaps.valid
            || (self.nodes[prod_idx].bcaps.datatype_mask & (1 << DATA_DMA_BUF)) != 0;

        let mut best = None;
        let mut has_mod = false;
        let mut prod_dont_fixate = false;

        if c_allows_dmabuf && p_allows_dmabuf {
            'p1: for pa in &self.nodes[prod_idx].vfmt_candidates {
                for pb in &self.nodes[cons_idx].vfmt_candidates {
                    if let Some(iv) = intersect_vfmt(pa, pb, true, lim_w, lim_h, lim_fps) {
                        best = Some(iv);
                        has_mod = true;
                        prod_dont_fixate = pa.mod_dont_fixate;
                        break 'p1;
                    }
                }
            }
        }
        if best.is_none() {
            'p2: for pa in &self.nodes[prod_idx].vfmt_candidates {
                for pb in &self.nodes[cons_idx].vfmt_candidates {
                    if !pa.has_modifier && !pb.has_modifier {
                        if let Some(iv) = intersect_vfmt(pa, pb, false, lim_w, lim_h, lim_fps) {
                            best = Some(iv);
                            break 'p2;
                        }
                    }
                }
            }
        }
        if best.is_none() {
            'p3: for pa in &self.nodes[prod_idx].vfmt_candidates {
                let mut ca = pa.clone();
                ca.has_modifier = false;
                for pb in &self.nodes[cons_idx].vfmt_candidates {
                    let mut cb = pb.clone();
                    cb.has_modifier = false;
                    if let Some(iv) = intersect_vfmt(&ca, &cb, false, lim_w, lim_h, lim_fps) {
                        best = Some(iv);
                        break 'p3;
                    }
                }
            }
        }
        if best.is_none() && self.nodes[cons_idx].vfmt_candidates.is_empty() {
            for pa in &self.nodes[prod_idx].vfmt_candidates {
                if !pa.has_modifier {
                    if let Some(iv) = intersect_vfmt(pa, pa, false, lim_w, lim_h, lim_fps) {
                        best = Some(iv);
                        break;
                    }
                }
            }
            if best.is_none() && c_allows_dmabuf {
                for pa in &self.nodes[prod_idx].vfmt_candidates {
                    if let Some(iv) =
                        intersect_vfmt(pa, pa, pa.has_modifier, lim_w, lim_h, lim_fps)
                    {
                        has_mod = pa.has_modifier;
                        prod_dont_fixate = has_mod && pa.mod_dont_fixate;
                        best = Some(iv);
                        break;
                    }
                }
            }
        }

        let Some(iv) = best else {
            let cc_pid = self.clients[cc_idx].pid;
            let cons_gid = self.nodes[cons_idx].global_id;
            let prod_gid = self.nodes[prod_idx].global_id;
            dbg_emit(
                LWASAPI_DBG_PW_VIDEO,
                LWASAPI_DBG_WARN,
                cc_pid,
                cons_gid,
                "PW_NEG_MISMATCH",
                &self.nodes[cons_idx].name,
                &format!(
                    "prod={}({}) nv={} vs cons={}({}) nv={}",
                    self.nodes[prod_idx].name,
                    prod_gid,
                    self.nodes[prod_idx].vfmt_candidates.len(),
                    self.nodes[cons_idx].name,
                    cons_gid,
                    self.nodes[cons_idx].vfmt_candidates.len()
                ),
            );
            return false;
        };

        let send_dont_fixate = has_mod && prod_dont_fixate && !self.links[li].mod_fixated;
        let mod_v = if has_mod {
            iv.mods.first().copied().unwrap_or(DRM_MOD_INVALID)
        } else {
            DRM_MOD_INVALID
        };
        let stride = if self.nodes[prod_idx].bcaps.stride_def != 0 {
            self.nodes[prod_idx].bcaps.stride_def
        } else {
            vfmt_stride(iv.fmt, iv.w)
        };
        if iv.w == 0 || iv.h == 0 || iv.w > 7680 || iv.h > 4320 {
            return false;
        }
        let sz = vfmt_size(iv.fmt, iv.w, iv.h).max(4096);
        if sz > 128 * 1024 * 1024 {
            return false;
        }

        for idx in [prod_idx, cons_idx] {
            let n = &mut self.nodes[idx];
            n.neg_format = iv.fmt;
            n.neg_w = iv.w;
            n.neg_h = iv.h;
            n.neg_stride = stride;
            n.neg_fps_num = iv.fps.num;
            n.neg_fps_denom = iv.fps.denom;
            n.neg_modifier = mod_v;
            n.neg_has_modifier = has_mod;
            n.neg_is_dmabuf = has_mod;
        }

        let pod = build_negotiated_format_pod_iv(&iv, has_mod, send_dont_fixate);
        if pod.len() < 8 {
            return false;
        }
        self.links[li].format_pod = pod.clone();

        let prod_cn = self.nodes[prod_idx].cn_proxy_id;
        let cons_cn = self.nodes[cons_idx].cn_proxy_id;

        if send_dont_fixate {
            self.links[li].awaiting_mod_fixation = true;
            self.links[li].state = 1;
            self.cn_send_port_set_param(pc_idx, prod_cn, PWB_DIR_OUT, 0, PARAM_FORMAT, &pod);
            return true;
        }

        self.links[li].awaiting_mod_fixation = false;
        self.cn_send_port_set_param(pc_idx, prod_cn, PWB_DIR_OUT, 0, PARAM_FORMAT, &pod);
        self.cn_send_port_set_param(cc_idx, cons_cn, PWB_DIR_IN, 0, PARAM_FORMAT, &pod);

        let prod_gid = self.nodes[prod_idx].global_id;
        let cons_gid = self.nodes[cons_idx].global_id;
        if !self.nodes[prod_idx].mix_ready {
            self.cn_send_port_set_mix_info(pc_idx, prod_cn, PWB_DIR_OUT, 0, 0, cons_gid);
            self.nodes[prod_idx].mix_ready = true;
        }
        if !self.nodes[cons_idx].mix_ready {
            self.cn_send_port_set_mix_info(cc_idx, cons_cn, PWB_DIR_IN, 0, 0, prod_gid);
            self.nodes[cons_idx].mix_ready = true;
        }

        let nbuf = if self.nodes[prod_idx].bcaps.valid {
            let bmax = self.nodes[prod_idx].bcaps.buf_max.clamp(2, 8);
            let bmin = self.nodes[prod_idx].bcaps.buf_min.clamp(2, bmax);
            let target = self.nodes[prod_idx].bcaps.buf_def.max(4);
            target.clamp(bmin, bmax) as usize
        } else {
            4
        };
        let msz = nbuf * PWB_META_STRIDE;
        let mut old_meta = self.nodes[prod_idx].meta_mem_id;
        self.clients[pc_idx].remove_mem(&mut old_meta);
        self.nodes[prod_idx].meta_mem_id = old_meta;
        self.nodes[prod_idx].meta_map = None;
        self.nodes[prod_idx].meta_mem_fd = None;

        let Some(pmeta_fd) = create_memfd("salsa-pw-pmeta", msz) else {
            return false;
        };
        let pmeta_map = MappedMem::map_fd(pmeta_fd.as_raw_fd(), msz, true, 0);
        seal_memfd_size(pmeta_fd.as_raw_fd());
        let Some(pmeta_mid) =
            self.clients[pc_idx].add_mem(DATA_MEM_FD, pmeta_fd.as_raw_fd(), 3)
        else {
            return false;
        };
        self.nodes[prod_idx].meta_mem_fd = Some(pmeta_fd);
        self.nodes[prod_idx].meta_map = pmeta_map;
        self.nodes[prod_idx].meta_mem_id = pmeta_mid;

        if self.nodes[prod_idx].alloc_buffers {
            let type_mask = if has_mod {
                (1 << DATA_DMA_BUF) | (1 << DATA_MEM_FD)
            } else {
                1 << DATA_MEM_FD
            };
            let items = vec![(type_mask, SPA_ID_INVALID, 0, 0, 0); nbuf];
            self.nodes[prod_idx].awaiting_port_buffers = true;
            self.nodes[prod_idx].pending_link_idx = li as u32;
            self.links[li].state = 2;
            self.cn_send_port_use_buffers(pc_idx, prod_idx, 0, 1, pmeta_mid, &items);
            return true;
        }

        let mut old_bids = self.nodes[prod_idx].buf_mem_ids;
        for id in &mut old_bids {
            self.clients[pc_idx].remove_mem(id);
        }
        self.nodes[prod_idx].buf_mem_ids = old_bids;
        self.nodes[prod_idx].free_buffers();

        let mut items = Vec::with_capacity(nbuf);
        for i in 0..nbuf {
            let Some(bfd) = create_memfd("salsa-pw-vbuf", sz as usize) else {
                return false;
            };
            let mp = MappedMem::map_fd(bfd.as_raw_fd(), sz as usize, true, 0);
            seal_memfd_size(bfd.as_raw_fd());
            let Some(pmid) = self.clients[pc_idx].add_mem(DATA_MEM_FD, bfd.as_raw_fd(), 3) else {
                return false;
            };
            self.nodes[prod_idx].buf_mem_ids[i] = pmid;
            self.nodes[prod_idx].bufs.push(NodeBuf {
                fd: Some(bfd),
                mem_type: DATA_MEM_FD,
                _flags: 11,
                mapoffset: 0,
                maxsize: sz,
                mapped: mp,
            });
            items.push((DATA_MEM_ID, pmid, 11, 0, sz));
        }
        self.nodes[prod_idx].n_bufs = nbuf as u32;
        self.cn_send_port_use_buffers(pc_idx, prod_idx, 0, 0, pmeta_mid, &items);
        self.complete_link_with_prod_buffers(li);
        true
    }

    fn try_autolink_all(&mut self) {
        for i in 0..PWB_MAX_NODES {
            if !self.nodes[i].in_use
                || self.nodes[i].direction != PWB_DIR_IN
                || !self.nodes[i].port_registered
                || self.node_is_linked(i, false)
            {
                continue;
            }
            let tgt_id = self.nodes[i].target_id_req;
            let tgt_str = self.nodes[i].target_str.clone();
            let auto = self.nodes[i].autoconnect;
            let mut best: Option<usize> = None;
            let mut best_is_src = false;
            let mut best_unlinked = false;

            for j in 0..PWB_MAX_NODES {
                let prod = &self.nodes[j];
                if !prod.in_use
                    || prod.direction != PWB_DIR_OUT
                    || !prod.port_registered
                    || prod.vfmt_candidates.is_empty()
                {
                    continue;
                }
                if tgt_id != 0 && tgt_id != SPA_ID_INVALID {
                    if prod.global_id == tgt_id || prod.serial == tgt_id {
                        best = Some(j);
                        break;
                    }
                } else if !tgt_str.is_empty() {
                    if parse_u32_strict(&tgt_str)
                        .is_some_and(|tid| prod.global_id == tid || prod.serial == tid)
                        || prod.name == tgt_str
                    {
                        best = Some(j);
                        break;
                    }
                } else if auto {
                    let is_src = prod.media_class.contains("Video/Source");
                    let unlinked = !self.node_is_linked(j, true);
                    if best.is_none()
                        || (is_src && !best_is_src)
                        || (is_src == best_is_src && unlinked && !best_unlinked)
                    {
                        best = Some(j);
                        best_is_src = is_src;
                        best_unlinked = unlinked;
                        if is_src && unlinked {
                            break;
                        }
                    }
                }
            }

            let Some(pidx) = best else {
                continue;
            };
            let Some(li) = self.links.iter().position(|l| !l.in_use) else {
                continue;
            };
            self.links[li] = PwLink {
                in_use: true,
                global_id: self.next_global_id,
                serial: self.next_serial,
                out_node_idx: pidx as u32,
                in_node_idx: i as u32,
                last_buf_id: SPA_ID_INVALID,
                ..PwLink::default()
            };
            self.next_global_id += 1;
            self.next_serial += 1;
            if !self.negotiate_and_activate_link(li) {
                self.links[li].in_use = false;
            }
        }
    }

    fn copy_frame_to_preview_shm(&mut self, nidx: usize, buf_id: u32, cons_count: u32) {
        let shm_base = match self.preview_shm_map.as_ref() {
            Some(m) => m.as_ptr().cast::<LwasapiPwPreviewShm>().cast_mut(),
            None => return,
        };
        let req = unsafe { atomic_u32_load(ptr::addr_of!((*shm_base).preview_requested), Ordering::Relaxed) };
        if req == 0 {
            self.dmabuf_blitter = None;
            return;
        }
        let now = mono_ns();
        let exp_ns = self.shared.preview_expiry_ns.load(Ordering::Relaxed);
        let last_ping =
            unsafe { atomic_u64_load(ptr::addr_of!((*shm_base).last_gui_ping_ns), Ordering::Relaxed) };
        let ping_fresh = last_ping > 0 && now.saturating_sub(last_ping) <= 8_000_000_000;
        if exp_ns > 0 && now > exp_ns {
            if ping_fresh {
                // Continuous SHM keepalive pings from the debugger keep the stream alive past initial TTL.
                self.shared.preview_expiry_ns.store(0, Ordering::Relaxed);
            } else {
                unsafe {
                    atomic_u32_store(
                        ptr::addr_of_mut!((*shm_base).preview_requested),
                        0,
                        Ordering::Relaxed,
                    );
                }
                self.dmabuf_blitter = None;
                return;
            }
        } else if exp_ns == 0 && last_ping > 0 && !ping_fresh {
            unsafe {
                atomic_u32_store(
                    ptr::addr_of_mut!((*shm_base).preview_requested),
                    0,
                    Ordering::Relaxed,
                );
            }
            self.dmabuf_blitter = None;
            return;
        }

        let (tw, th_lim, tfps) = if ping_fresh {
            let sw_t = unsafe { atomic_u32_load(ptr::addr_of!((*shm_base).target_max_w), Ordering::Relaxed) };
            let sh_t = unsafe { atomic_u32_load(ptr::addr_of!((*shm_base).target_max_h), Ordering::Relaxed) };
            let sf_t = unsafe { atomic_u32_load(ptr::addr_of!((*shm_base).target_fps), Ordering::Relaxed) };
            let cw = sw_t.min(LWASAPI_PW_PREVIEW_MAX_W);
            let ch = sh_t.min(LWASAPI_PW_PREVIEW_MAX_H);
            let cf = sf_t.min(240);
            self.shared.preview_target_w.store(cw, Ordering::Relaxed);
            self.shared.preview_target_h.store(ch, Ordering::Relaxed);
            self.shared.preview_target_fps.store(cf, Ordering::Relaxed);
            (cw, ch, cf)
        } else {
            (
                self.shared.preview_target_w.load(Ordering::Relaxed),
                self.shared.preview_target_h.load(Ordering::Relaxed),
                self.shared.preview_target_fps.load(Ordering::Relaxed),
            )
        };
        if tfps != self.preview_applied_fps {
            self.preview_next_due_ns = 0;
            self.preview_fps_win_ns = 0;
            self.preview_fps_win_cnt = 0;
            self.preview_applied_fps = tfps;
        }
        if !pace_due(now, &mut self.preview_next_due_ns, tfps) {
            return;
        }
        update_fps_meter(
            now,
            &mut self.preview_fps_win_ns,
            &mut self.preview_fps_win_cnt,
            &mut self.preview_actual_fps_x100,
        );
        self.shared
            .preview_actual_fps_x100_pub
            .store(self.preview_actual_fps_x100, Ordering::Relaxed);

        let prod = &mut self.nodes[nidx];
        let bidx = buf_id as usize;
        if bidx >= prod.bufs.len() || prod.neg_w == 0 || prod.neg_h == 0 {
            return;
        }
        let sw = prod.neg_w;
        let mut sh = prod.neg_h;
        let neg_fmt = prod.neg_format;
        let neg_stride = prod.neg_stride;
        let neg_mod = prod.neg_modifier;
        let neg_has_mod = prod.neg_has_modifier;
        let prod_gid = prod.global_id;
        let prod_name = prod.name.clone();

        let (mut coff, mut cstride) = (0u32, 0i32);
        if let Some(mmap) = prod.meta_map.as_ref() {
            let off = bidx * PWB_META_STRIDE;
            let sl = mmap.as_slice();
            if off + PWB_META_TOTAL_SZ <= sl.len() {
                let co = off + PWB_META_CHUNK_OFF;
                coff = u32::from_le_bytes([sl[co], sl[co + 1], sl[co + 2], sl[co + 3]]);
                cstride = i32::from_le_bytes([
                    sl[co + 8],
                    sl[co + 9],
                    sl[co + 10],
                    sl[co + 11],
                ]);
            }
        }

        let pb = &mut prod.bufs[bidx];
        if pb.mapped.is_none()
            && pb.maxsize > 0
            && pb.maxsize <= 128 * 1024 * 1024
            && (pb.mem_type == DATA_MEM_FD
                || (pb.mem_type == DATA_DMA_BUF && (!neg_has_mod || neg_mod == 0)))
        {
            if let Some(f) = pb.fd.as_ref() {
                let size_ok = if pb.mem_type == DATA_MEM_FD {
                    memfd_size_valid(f.as_raw_fd(), pb.mapoffset as u64, pb.maxsize as u64)
                } else {
                    true
                };
                if size_ok {
                    pb.mapped = MappedMem::map_fd(
                        f.as_raw_fd(),
                        pb.maxsize as usize,
                        false,
                        pb.mapoffset as u64,
                    );
                }
            }
        }
        let is_dmabuf = pb.mem_type == DATA_DMA_BUF;
        let cpu_mappable = pb.mapped.is_some();

        let seq = unsafe { atomic_u64_load(ptr::addr_of!((*shm_base).seqlock), Ordering::Relaxed) };
        unsafe {
            atomic_u64_store(ptr::addr_of_mut!((*shm_base).seqlock), seq + 1, Ordering::Release);
            let s = &mut *shm_base;
            s.active_stream = 1;
            s.is_dmabuf = u32::from(is_dmabuf);
            s.dmabuf_mappable = u32::from(is_dmabuf && cpu_mappable);
            s.src_width = sw;
            s.src_height = sh;
            s.src_stride = neg_stride;
            s.src_spa_format = neg_fmt;
            s.drm_modifier_lo = (neg_mod & 0xffff_ffff) as u32;
            s.drm_modifier_hi = (neg_mod >> 32) as u32;
            s.prod_node_id = prod_gid;
            s.cons_count = cons_count;
            s.frame_seq = s.frame_seq.wrapping_add(1);
            s.pts_ns = now;
            copy_str_to_c(&mut s.prod_name, &prod_name);
            copy_str_to_c(&mut s.format_name, vfmt_name(neg_fmt));
            s.target_max_w = tw;
            s.target_max_h = th_lim;
            s.target_fps = tfps;
            s.actual_fps_x100 = if self.relay_actual_fps_x100 > 0 {
                self.relay_actual_fps_x100
            } else {
                self.preview_actual_fps_x100
            };
            s.cast_fps_cap = self.shared.cast_limit_fps.load(Ordering::Relaxed);
        }

        let cap_w = if tw > 0 && tw <= LWASAPI_PW_PREVIEW_MAX_W {
            tw
        } else {
            LWASAPI_PW_PREVIEW_MAX_W
        };
        let cap_h = if th_lim > 0 && th_lim <= LWASAPI_PW_PREVIEW_MAX_H {
            th_lim
        } else {
            LWASAPI_PW_PREVIEW_MAX_H
        };
        let mut dw = sw;
        let mut dh = sh;
        if dw > cap_w {
            dh = ((dh as u64 * cap_w as u64) / dw as u64) as u32;
            dw = cap_w;
        }
        if dh > cap_h {
            dw = ((dw as u64 * cap_h as u64) / dh as u64) as u32;
            dh = cap_h;
        }
        dw = dw.max(1);
        dh = dh.max(1);
        let min_row = vfmt_stride(neg_fmt, sw) as usize;
        let sstride = if cstride > 0 {
            cstride as usize
        } else if neg_stride != 0 {
            neg_stride as usize
        } else {
            min_row
        };

        if let Some(src_map) = pb.mapped.as_ref() {
            if is_dmabuf {
                if let Some(f) = pb.fd.as_ref() {
                    dmabuf_sync(f.as_raw_fd(), true);
                }
            }
            let full = src_map.as_slice();
            let coff_u = if (coff as usize) < full.len() {
                coff as usize
            } else {
                0
            };
            let src = &full[coff_u..];
            if sstride < min_row || min_row == 0 || src.len() < min_row || src.len() < sstride {
                sh = 0;
            } else if (sh as usize).saturating_mul(sstride) > src.len() {
                sh = (src.len() / sstride) as u32;
            }
            let dst_stride = dw * 4;
            unsafe {
                let s = &mut *shm_base;
                s.dst_width = dw;
                s.dst_height = dh;
                s.dst_stride = dst_stride;
                if sh > 0 {
                    for y in 0..dh {
                        let sy = ((y as u64 * sh as u64) / dh as u64) as u32;
                        let roff = (y as usize) * (dst_stride as usize);
                        let rend = roff + (dst_stride as usize);
                        if let Some(drow) = s.pixels.get_mut(roff..rend) {
                            convert_preview_row(neg_fmt, src, sy, sh, sw, sstride, drow, dw);
                        }
                    }
                }
            }
            if is_dmabuf {
                if let Some(f) = pb.fd.as_ref() {
                    dmabuf_sync(f.as_raw_fd(), false);
                }
            }
        } else if is_dmabuf {
            let dma_fd = pb.fd.as_ref().map_or(-1, AsRawFd::as_raw_fd);
            let dma_off = pb.mapoffset.saturating_add(coff);
            let blitter = self.dmabuf_blitter.get_or_insert_with(DmabufGlBlitter::new);
            let dst_stride = dw * 4;
            let dst_len = (dh as usize).saturating_mul(dst_stride as usize);
            let desc = DmabufFrameDesc {
                dmabuf_fd: dma_fd,
                sw,
                sh,
                offset: dma_off,
                stride: sstride as u32,
                spa_fmt: neg_fmt,
                modifier: neg_has_mod.then_some(neg_mod),
                dw,
                dh,
            };
            let gl_ok = unsafe {
                let s = &mut *shm_base;
                s.pixels
                    .get_mut(..dst_len)
                    .is_some_and(|dst| blitter.blit_to_bgrx(&desc, dst))
            };
            unsafe {
                let s = &mut *shm_base;
                if gl_ok {
                    s.dmabuf_mappable = 1;
                    s.dst_width = dw;
                    s.dst_height = dh;
                    s.dst_stride = dst_stride;
                } else {
                    s.dst_width = 0;
                    s.dst_height = 0;
                    s.dst_stride = 0;
                }
            }
        } else {
            unsafe {
                let s = &mut *shm_base;
                s.dst_width = 0;
                s.dst_height = 0;
                s.dst_stride = 0;
            }
        }

        unsafe {
            atomic_u64_store(ptr::addr_of_mut!((*shm_base).seqlock), seq + 2, Ordering::Release);
        }
    }

    fn on_prod_done_event(&mut self, nidx: usize) {
        if nidx >= PWB_MAX_NODES || !self.nodes[nidx].in_use {
            return;
        }
        if let Some(efd) = self.nodes[nidx].done_efd.as_ref() {
            eventfd_drain(efd.as_raw_fd());
        }
        if let Some(dmap) = self.nodes[nidx].done_act_map.as_mut() {
            let sl = dmap.as_mut_slice();
            if sl.len() >= 20 {
                sl[8..12].copy_from_slice(&0i32.to_le_bytes());
                sl[12..16].copy_from_slice(&1i32.to_le_bytes());
                sl[16..20].copy_from_slice(&1i32.to_le_bytes());
                atomic_u32_store(sl.as_mut_ptr().cast::<u32>(), 0, Ordering::Release);
            }
        }
        let Some((chosen_slot, buf_id)) = self.nodes[nidx].io_take_ready_slot() else {
            return;
        };
        if let Some(awfd) = self.nodes[nidx].act_writefd.as_ref() {
            eventfd_drain(awfd.as_raw_fd());
        }
        self.nodes[nidx].frames_count += 1;
        let now_ns = mono_ns();

        let cl_fps = self.shared.cast_limit_fps.load(Ordering::Relaxed);
        if cl_fps != self.nodes[nidx].relay_applied_fps {
            self.nodes[nidx].relay_next_due_ns = 0;
            self.nodes[nidx].relay_applied_fps = cl_fps;
        }
        if cl_fps > 0
            && self
                .links
                .iter()
                .any(|lk| lk.in_use && lk.out_node_idx as usize == nidx && lk.io_ready)
            && !pace_due(now_ns, &mut self.nodes[nidx].relay_next_due_ns, cl_fps)
        {
            self.nodes[nidx].io_write_slot(chosen_slot, PWB_STATUS_NEED_DATA, buf_id);
            return;
        }

        let prod_seq = self.nodes[nidx].frames_count;
        let mut meta_pkt = None;
        if let Some(pmap) = self.nodes[nidx].meta_map.as_ref() {
            let off = (buf_id as usize) * PWB_META_STRIDE;
            let sl = pmap.as_slice();
            if off + PWB_META_TOTAL_SZ <= sl.len() {
                let mut h = [0u8; 32];
                let mut crop = [0u8; 16];
                let mut ch = [0u8; 16];
                h.copy_from_slice(&sl[off + PWB_META_HDR_OFF..off + PWB_META_HDR_OFF + 32]);
                crop.copy_from_slice(&sl[off + PWB_META_CROP_OFF..off + PWB_META_CROP_OFF + 16]);
                ch.copy_from_slice(&sl[off + PWB_META_CHUNK_OFF..off + PWB_META_CHUNK_OFF + 16]);
                let pts = i64::from_le_bytes([
                    h[8], h[9], h[10], h[11], h[12], h[13], h[14], h[15],
                ]);
                if pts <= 0 {
                    h[8..16].copy_from_slice(&(now_ns as i64).to_le_bytes());
                }
                let seq = u64::from_le_bytes([
                    h[24], h[25], h[26], h[27], h[28], h[29], h[30], h[31],
                ]);
                if seq == 0 {
                    h[24..32].copy_from_slice(&prod_seq.to_le_bytes());
                }
                meta_pkt = Some((h, crop, ch));
            }
        }

        let mut cons_woken = 0u32;
        let mut stale_to_recycle: Option<u32> = None;
        for li in 0..PWB_MAX_LINKS {
            if !self.links[li].in_use
                || self.links[li].out_node_idx as usize != nidx
                || !self.links[li].io_ready
            {
                continue;
            }
            let cidx = self.links[li].in_node_idx as usize;
            if cidx >= PWB_MAX_NODES
                || !self.nodes[cidx].in_use
                || self.nodes[cidx].io_map.is_none()
                || self.nodes[cidx].act_readfd.is_none()
            {
                continue;
            }
            if let Some((cst, prev_buf)) = self.nodes[cidx].io_read_status_and_buf() {
                if (cst == PWB_STATUS_HAVE_DATA || cst == PWB_STATUS_NEED_DATA)
                    && prev_buf != SPA_ID_INVALID
                    && prev_buf != buf_id
                {
                    stale_to_recycle = Some(prev_buf);
                    if cst == PWB_STATUS_HAVE_DATA {
                        self.shared.frames_dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            if let (Some((h, crop, ch)), Some(cmap)) =
                (meta_pkt, self.nodes[cidx].meta_map.as_mut())
            {
                let off = (buf_id as usize) * PWB_META_STRIDE;
                let csl = cmap.as_mut_slice();
                if off + PWB_META_TOTAL_SZ <= csl.len() {
                    csl[off + PWB_META_HDR_OFF..off + PWB_META_HDR_OFF + 32].copy_from_slice(&h);
                    csl[off + PWB_META_CROP_OFF..off + PWB_META_CROP_OFF + 16]
                        .copy_from_slice(&crop);
                    csl[off + PWB_META_CHUNK_OFF..off + PWB_META_CHUNK_OFF + 16]
                        .copy_from_slice(&ch);
                }
            }

            self.nodes[cidx].io_write_status_and_buf(PWB_STATUS_HAVE_DATA, buf_id);
            self.links[li].last_buf_id = buf_id;
            self.links[li].frames_relayed += 1;
            self.nodes[cidx].frames_count += 1;

            self.nodes[cidx].stamp_activation_clock(now_ns);
            if let Some(amap) = self.nodes[cidx].act_map.as_mut() {
                let asl = amap.as_mut_slice();
                if asl.len() >= 20 {
                    asl[8..12].copy_from_slice(&0i32.to_le_bytes());
                    asl[12..16].copy_from_slice(&1i32.to_le_bytes());
                    asl[16..20].copy_from_slice(&0i32.to_le_bytes());
                    atomic_u32_store(asl.as_mut_ptr().cast::<u32>(), 1, Ordering::Release);
                }
            }
            if let Some(rfd) = self.nodes[cidx].act_readfd.as_ref() {
                eventfd_signal(rfd.as_raw_fd());
            }
            cons_woken += 1;
        }

        if let Some(prev_buf) = stale_to_recycle {
            let still_held = (0..PWB_MAX_LINKS).any(|li| {
                self.links[li].in_use
                    && self.links[li].out_node_idx as usize == nidx
                    && self.links[li].io_ready
                    && self
                        .nodes
                        .get(self.links[li].in_node_idx as usize)
                        .and_then(|cn| cn.io_read_status_and_buf())
                        .is_some_and(|(s, b)| s == PWB_STATUS_HAVE_DATA && b == prev_buf)
            });
            if !still_held {
                self.nodes[nidx].io_recycle_buf(prev_buf);
            }
        }

        if cons_woken > 0 {
            self.shared
                .frames_relayed_total
                .fetch_add(1, Ordering::Relaxed);
            update_fps_meter(
                now_ns,
                &mut self.relay_fps_win_ns,
                &mut self.relay_fps_win_cnt,
                &mut self.relay_actual_fps_x100,
            );
            self.relay_last_frame_ns = now_ns;
            self.copy_frame_to_preview_shm(nidx, buf_id, cons_woken);
        } else {
            self.nodes[nidx].io_write_slot(chosen_slot, PWB_STATUS_NEED_DATA, buf_id);
        }
    }

    fn on_cons_done_event(&mut self, nidx: usize) {
        if nidx >= PWB_MAX_NODES || !self.nodes[nidx].in_use {
            return;
        }
        if let Some(efd) = self.nodes[nidx].done_efd.as_ref() {
            eventfd_drain(efd.as_raw_fd());
        }
        if let Some(awfd) = self.nodes[nidx].act_writefd.as_ref() {
            eventfd_drain(awfd.as_raw_fd());
        }
        if let Some(dmap) = self.nodes[nidx].done_act_map.as_mut() {
            let sl = dmap.as_mut_slice();
            if sl.len() >= 20 {
                sl[8..12].copy_from_slice(&0i32.to_le_bytes());
                sl[12..16].copy_from_slice(&1i32.to_le_bytes());
                sl[16..20].copy_from_slice(&1i32.to_le_bytes());
                atomic_u32_store(sl.as_mut_ptr().cast::<u32>(), 0, Ordering::Release);
            }
        }
        let Some((st, rec_id)) = self.nodes[nidx].io_read_status_and_buf() else {
            return;
        };
        if st != PWB_STATUS_NEED_DATA || rec_id == SPA_ID_INVALID {
            return;
        }
        self.nodes[nidx].io_write_status(PWB_STATUS_OK);
        for li in 0..PWB_MAX_LINKS {
            if !self.links[li].in_use
                || self.links[li].in_node_idx as usize != nidx
                || !self.links[li].io_ready
            {
                continue;
            }
            let pidx = self.links[li].out_node_idx as usize;
            if pidx >= PWB_MAX_NODES || !self.nodes[pidx].in_use || self.nodes[pidx].io_map.is_none()
            {
                continue;
            }
            let still_held = (0..PWB_MAX_LINKS).any(|other_li| {
                other_li != li
                    && self.links[other_li].in_use
                    && self.links[other_li].out_node_idx as usize == pidx
                    && self.links[other_li].io_ready
                    && self
                        .nodes
                        .get(self.links[other_li].in_node_idx as usize)
                        .and_then(|cn| cn.io_read_status_and_buf())
                        .is_some_and(|(s, b)| s == PWB_STATUS_HAVE_DATA && b == rec_id)
            });
            if !still_held {
                // Match C broker behavior: recycle producer's io_map slot without
                // triggering prod->act_readfd out-of-phase between compositor vblanks.
                self.nodes[pidx].io_recycle_buf(rec_id);
            }
        }
    }

    fn send_core_info_and_client_bound(&mut self, ci: usize) {
        let user = current_username();
        let host = current_hostname();
        let cgid = self.clients[ci].global_id;
        let cprops = self.clients[ci].props.clone();

        let mut props = Dict::default();
        props.set_str("core.name", "pipewire-0");
        props.set_str("core.daemon", "true");
        props.set_str("application.name", "Salsa Video Broker");
        props.set_str("default.clock.rate", "48000");
        props.set_str("link.max-buffers", "16");

        let mut b = PodBuilder::new(2048);
        b.push_struct();
        b.int(0);
        b.int((0x534C_5057u32 ^ cgid) as i32);
        b.str_val(Some(&user));
        b.str_val(Some(&host));
        b.str_val(Some("1.6.11"));
        b.str_val(Some("pipewire-0"));
        b.long(1);
        b.dict(&props);
        b.pop();
        let info_pay = b.into_vec();
        let _ = self.clients[ci].send_msg(0, PW_CORE_E_INFO, &info_pay, &[]);

        let mut bp = PodBuilder::new(2048);
        bp.push_struct();
        bp.int(1);
        bp.int(cgid as i32);
        bp.dict(&cprops);
        bp.pop();
        let bp_pay = bp.into_vec();
        let _ = self.clients[ci].send_msg(0, PW_CORE_E_BOUND_PROPS, &bp_pay, &[]);
    }

    fn handle_core_msg(&mut self, ci: usize, op: u8, payload: &[u8]) {
        let Some(mut cur) = PodCursor::new(payload, 0, payload.len()) else {
            return;
        };
        let Some(mut st) = cur.enter_struct_cursor() else {
            return;
        };

        match op {
            PW_CORE_M_HELLO => {
                let re_hello = self.clients[ci].hello_done;
                self.clients[ci].hello_done = true;
                if re_hello {
                    self.clients[ci].objs.fill(PwProxy::default());
                }
                self.send_core_info_and_client_bound(ci);
                if !re_hello {
                    let cgid = self.clients[ci].global_id;
                    let cprops = self.clients[ci].props.clone();
                    self.broadcast_global(cgid, "PipeWire:Interface:Client", 3, &cprops);
                }
            }
            PW_CORE_M_SYNC => {
                let (Some(id), Some(sseq)) = (st.read_u32(), st.read_u32()) else {
                    return;
                };
                let mut b = PodBuilder::new(64);
                b.push_struct();
                b.int(id as i32);
                b.int(sseq as i32);
                b.pop();
                let pay = b.into_vec();
                let _ = self.clients[ci].send_msg(0, PW_CORE_E_DONE, &pay, &[]);
            }
            PW_CORE_M_GET_REGISTRY => {
                let (Some(_ver), Some(new_id)) = (st.read_u32(), st.read_u32()) else {
                    return;
                };
                self.clients[ci].alloc_proxy(new_id, 0, PROXY_KIND_REGISTRY);

                let emit_reg = |c: &mut PwClient, gid: u32, iface: &str, ver: u32, d: &Dict| {
                    let mut b = PodBuilder::new(2048);
                    b.push_struct();
                    b.int(gid as i32);
                    b.int(PWB_PERM_ALL as i32);
                    b.str_val(Some(iface));
                    b.int(ver as i32);
                    b.dict(d);
                    b.pop();
                    let pay = b.into_vec();
                    let _ = c.send_msg(new_id, PW_REG_E_GLOBAL, &pay, &[]);
                };

                let mut d0 = Dict::default();
                d0.set_str("core.name", "pipewire-0");
                d0.set_str("object.serial", "0");
                emit_reg(&mut self.clients[ci], 0, "PipeWire:Interface:Core", 4, &d0);

                for fi in 1..=4 {
                    let (_, _, _, fd) = build_factory_dict(fi);
                    emit_reg(
                        &mut self.clients[ci],
                        fi,
                        "PipeWire:Interface:Factory",
                        3,
                        &fd,
                    );
                }
                for mi in 5..=6 {
                    let md = build_metadata_dict(mi);
                    emit_reg(
                        &mut self.clients[ci],
                        mi,
                        "PipeWire:Interface:Metadata",
                        3,
                        &md,
                    );
                }
                let client_globals: Vec<(u32, Dict)> = self
                    .clients
                    .iter()
                    .filter(|c| c.in_use && c.hello_done)
                    .map(|c| (c.global_id, c.props.clone()))
                    .collect();
                for (gid, props) in client_globals {
                    emit_reg(
                        &mut self.clients[ci],
                        gid,
                        "PipeWire:Interface:Client",
                        3,
                        &props,
                    );
                }
                let node_globals: Vec<(u32, Dict, bool, u32, Dict)> = self
                    .nodes
                    .iter()
                    .filter(|n| n.in_use)
                    .map(|n| {
                        (
                            n.global_id,
                            n.build_public_props(),
                            n.port_registered,
                            n.port_global_id,
                            n.port_props.clone(),
                        )
                    })
                    .collect();
                for (gid, ndd, preg, pgid, pprops) in node_globals {
                    emit_reg(
                        &mut self.clients[ci],
                        gid,
                        "PipeWire:Interface:Node",
                        3,
                        &ndd,
                    );
                    if preg {
                        emit_reg(
                            &mut self.clients[ci],
                            pgid,
                            "PipeWire:Interface:Port",
                            3,
                            &pprops,
                        );
                    }
                }
                let link_globals: Vec<(u32, Dict)> = self
                    .links
                    .iter()
                    .filter(|l| l.in_use)
                    .map(|l| {
                        let mut ld = Dict::default();
                        ld.set_str(
                            "link.output.node",
                            &self.nodes[l.out_node_idx as usize].global_id.to_string(),
                        );
                        ld.set_str(
                            "link.input.node",
                            &self.nodes[l.in_node_idx as usize].global_id.to_string(),
                        );
                        (l.global_id, ld)
                    })
                    .collect();
                for (gid, ld) in link_globals {
                    emit_reg(
                        &mut self.clients[ci],
                        gid,
                        "PipeWire:Interface:Link",
                        3,
                        &ld,
                    );
                }
            }
            PW_CORE_M_CREATE_OBJ => {
                let Some(fac_b) = st.read_str_bytes() else {
                    return;
                };
                let fac = String::from_utf8_lossy(fac_b).into_owned();
                let Some(tp_b) = st.read_str_bytes() else {
                    return;
                };
                let tp = String::from_utf8_lossy(tp_b).into_owned();
                let Some(_ver) = st.read_u32() else {
                    return;
                };
                let mut props = Dict::default();
                if st.read_dict(&mut props, true).is_none() {
                    return;
                }
                let Some(new_id) = st.read_u32() else {
                    return;
                };

                if fac == "client-node" || tp.contains("ClientNode") {
                    let mtype = props.get_str("media.type").unwrap_or_default();
                    let mclass = props.get_str("media.class").unwrap_or_default();
                    if mtype.eq_ignore_ascii_case("Audio") || mclass.contains("Audio") {
                        self.shared.audio_rejects.fetch_add(1, Ordering::Relaxed);
                        let mut eb = PodBuilder::new(256);
                        eb.push_struct();
                        eb.int(new_id as i32);
                        eb.int(0);
                        eb.int(-93); // -EPROTONOSUPPORT
                        eb.str_val(Some(
                            "Salsa pipewire-0 broker is Video-Only (audio routes via Salsa PA/ALSA/WASAPI)",
                        ));
                        eb.pop();
                        let ep = eb.into_vec();
                        let _ = self.clients[ci].send_msg(0, PW_CORE_E_ERROR, &ep, &[]);
                        let pid = self.clients[ci].pid;
                        let app = self.clients[ci].app_name.clone();
                        dbg_emit(
                            LWASAPI_DBG_PW_VIDEO,
                            LWASAPI_DBG_WARN,
                            pid,
                            new_id,
                            "PW_AUDIO_REJECTED",
                            if mclass.is_empty() { "Audio" } else { &mclass },
                            &format!("client={} rejected audio node on video-only broker", app),
                        );
                        return;
                    }

                    let Some(free_i) = self.nodes.iter().position(|n| !n.in_use) else {
                        return;
                    };
                    let ngid = self.next_global_id;
                    let nser = self.next_serial;
                    self.next_global_id += 1;
                    self.next_serial += 1;

                    let drv = props.get_str("node.driver").as_deref() == Some("true");
                    let ac = props.get_str("node.autoconnect");
                    let is_drv = drv || mclass.contains("Video/Source");
                    let auto = ac.as_deref().is_none_or(|v| v == "true");
                    let dir = if mclass.contains("Input") || mclass.contains("Sink") {
                        PWB_DIR_IN
                    } else {
                        PWB_DIR_OUT
                    };
                    let name = props
                        .get_str("node.name")
                        .or_else(|| props.get_str("media.name"))
                        .unwrap_or_else(|| self.clients[ci].app_name.clone());

                    let mut nd = PwNode {
                        in_use: true,
                        is_driver: is_drv,
                        autoconnect: auto,
                        direction: dir,
                        state: 2,
                        global_id: ngid,
                        serial: nser,
                        client_idx: ci as u32,
                        cn_proxy_id: new_id,
                        name,
                        media_class: if mclass.is_empty() {
                            "Video".to_string()
                        } else {
                            mclass
                        },
                        node_props: props,
                        ..PwNode::default()
                    };
                    nd.apply_target_props();

                    let Some(act_fd) = create_memfd("salsa-pw-act", PWB_ACT_SIZE) else {
                        return;
                    };
                    let mut act_map = MappedMem::map_fd(act_fd.as_raw_fd(), PWB_ACT_SIZE, true, 0);
                    seal_memfd_size(act_fd.as_raw_fd());
                    if let Some(m) = act_map.as_mut() {
                        let sl = m.as_mut_slice();
                        sl[0..4].copy_from_slice(&3u32.to_le_bytes());
                        sl[8..12].copy_from_slice(&0i32.to_le_bytes());
                        sl[12..16].copy_from_slice(&1i32.to_le_bytes());
                        sl[16..20].copy_from_slice(&1i32.to_le_bytes());
                    }
                    let act_read = new_eventfd();
                    let act_write = new_eventfd();
                    let done_efd = new_eventfd();
                    let done_act_fd = create_memfd("salsa-pw-dact", PWB_ACT_SIZE);
                    if let Some(df) = done_act_fd.as_ref() {
                        seal_memfd_size(df.as_raw_fd());
                    }
                    let mut done_act_map = done_act_fd
                        .as_ref()
                        .and_then(|f| MappedMem::map_fd(f.as_raw_fd(), PWB_ACT_SIZE, true, 0));
                    if let Some(dm) = done_act_map.as_mut() {
                        let sl = dm.as_mut_slice();
                        sl[544..548].copy_from_slice(&1u32.to_le_bytes());
                        sl[8..12].copy_from_slice(&0i32.to_le_bytes());
                        sl[12..16].copy_from_slice(&1i32.to_le_bytes());
                        sl[16..20].copy_from_slice(&1i32.to_le_bytes());
                    }
                    nd.act_map = act_map;
                    nd.update_activation_pos(ngid);
                    nd.act_readfd = act_read;
                    nd.act_writefd = act_write;
                    nd.done_efd = done_efd;
                    nd.done_act_fd = done_act_fd;
                    nd.done_act_map = done_act_map;
                    let act_mem_id = self.clients[ci]
                        .add_mem(DATA_MEM_FD, act_fd.as_raw_fd(), 3)
                        .unwrap_or(SPA_ID_INVALID);
                    nd.act_mem_id = act_mem_id;
                    nd.act_mem_fd = Some(act_fd);

                    let ar_raw = nd.act_readfd.as_ref().map(AsRawFd::as_raw_fd);
                    let aw_raw = nd.act_writefd.as_ref().map(AsRawFd::as_raw_fd);
                    let ndpub = nd.build_public_props();
                    let nd_name = nd.name.clone();
                    let nd_mc = nd.media_class.clone();
                    self.nodes[free_i] = nd;

                    self.clients[ci].alloc_proxy(new_id, ngid, PROXY_KIND_CLIENT_NODE);
                    self.cn_send_set_activation(
                        ci,
                        new_id,
                        ngid,
                        ar_raw,
                        act_mem_id,
                        0,
                        PWB_ACT_SIZE as u32,
                    );

                    let mut bp = PodBuilder::new(2048);
                    bp.push_struct();
                    bp.int(new_id as i32);
                    bp.int(ngid as i32);
                    bp.dict(&ndpub);
                    bp.pop();
                    let bp_pay = bp.into_vec();
                    let _ = self.clients[ci].send_msg(0, PW_CORE_E_BOUND_PROPS, &bp_pay, &[]);

                    if let (Some(rf), Some(wf)) = (ar_raw, aw_raw) {
                        let mut tr = PodBuilder::new(128);
                        tr.push_struct();
                        tr.fd(0);
                        tr.fd(1);
                        tr.int(act_mem_id as i32);
                        tr.int(0);
                        tr.int(PWB_ACT_SIZE as i32);
                        tr.pop();
                        let tr_pay = tr.into_vec();
                        let _ = self.clients[ci]
                            .send_msg(new_id, PW_CN_E_TRANSPORT, &tr_pay, &[rf, wf]);
                    }

                    for (io_id, io_sz) in [
                        (IO_CLOCK, PWB_ACT_CLOCK_SIZE as i32),
                        (IO_POSITION, PWB_ACT_POS_SIZE as i32),
                    ] {
                        let mut io = PodBuilder::new(128);
                        io.push_struct();
                        io.id(io_id);
                        io.int(act_mem_id as i32);
                        io.int(PWB_ACT_POS_OFFSET as i32);
                        io.int(io_sz);
                        io.pop();
                        let io_pay = io.into_vec();
                        let _ = self.clients[ci].send_msg(new_id, PW_CN_E_SET_IO, &io_pay, &[]);
                    }

                    self.broadcast_global(ngid, "PipeWire:Interface:Node", 3, &ndpub);
                    let pid = self.clients[ci].pid;
                    let app = self.clients[ci].app_name.clone();
                    dbg_emit(
                        LWASAPI_DBG_PW_VIDEO,
                        LWASAPI_DBG_INFO,
                        pid,
                        ngid,
                        "PW_NODE_CREATED",
                        &nd_name,
                        &format!(
                            "class={} dir={} driver={} app={}",
                            nd_mc,
                            if dir != 0 { "OUT" } else { "IN" },
                            i32::from(is_drv),
                            app
                        ),
                    );
                } else if fac == "link-factory" || tp.contains("Link") {
                    let os = props
                        .get_str("link.output.node")
                        .filter(|s| !s.is_empty())
                        .or_else(|| props.get_str("link.output.port"))
                        .unwrap_or_default();
                    let is = props
                        .get_str("link.input.node")
                        .filter(|s| !s.is_empty())
                        .or_else(|| props.get_str("link.input.port"))
                        .unwrap_or_default();
                    if let (Some(oi), Some(ii)) = (
                        self.find_node_by_ref(&os, PWB_DIR_OUT),
                        self.find_node_by_ref(&is, PWB_DIR_IN),
                    ) {
                        if self.nodes[oi].direction == PWB_DIR_OUT
                            && self.nodes[ii].direction == PWB_DIR_IN
                        {
                            if let Some(li) = self.links.iter().position(|l| !l.in_use) {
                                let lgid = self.next_global_id;
                                let lser = self.next_serial;
                                self.next_global_id += 1;
                                self.next_serial += 1;
                                self.links[li] = PwLink {
                                    in_use: true,
                                    global_id: lgid,
                                    serial: lser,
                                    out_node_idx: oi as u32,
                                    in_node_idx: ii as u32,
                                    owner_client_idx: ci as u32,
                                    _owner_proxy_id: new_id,
                                    last_buf_id: SPA_ID_INVALID,
                                    ..PwLink::default()
                                };
                                self.clients[ci].alloc_proxy(new_id, lgid, PROXY_KIND_LINK);
                                let mut bp = PodBuilder::new(1024);
                                bp.push_struct();
                                bp.int(new_id as i32);
                                bp.int(lgid as i32);
                                bp.dict(&props);
                                bp.pop();
                                let bp_pay = bp.into_vec();
                                let _ =
                                    self.clients[ci].send_msg(0, PW_CORE_E_BOUND_PROPS, &bp_pay, &[]);
                                if !self.negotiate_and_activate_link(li) {
                                    self.links[li].in_use = false;
                                }
                            }
                        }
                    }
                }
            }
            PW_CORE_M_DESTROY => {
                let Some(id) = st.read_u32() else {
                    return;
                };
                if let Some(px) = self.clients[ci].find_proxy(id) {
                    if px.kind == PROXY_KIND_CLIENT_NODE {
                        if let Some(nidx) = self.find_node_by_global(px.global_id) {
                            self.destroy_node(nidx);
                        }
                    } else if px.kind == PROXY_KIND_LINK {
                        for li in 0..PWB_MAX_LINKS {
                            if self.links[li].in_use && self.links[li].global_id == px.global_id {
                                self.destroy_link(li);
                            }
                        }
                    }
                    if let Some(slot) = self.clients[ci]
                        .objs
                        .iter_mut()
                        .find(|o| o.in_use && o.proxy_id == id)
                    {
                        slot.in_use = false;
                    }
                }
                self.clients[ci].send_remove_id(id);
            }
            _ => {}
        }
    }

    fn handle_registry_msg(&mut self, ci: usize, op: u8, payload: &[u8]) {
        let Some(mut cur) = PodCursor::new(payload, 0, payload.len()) else {
            return;
        };
        let Some(mut st) = cur.enter_struct_cursor() else {
            return;
        };
        if op == PW_REG_M_BIND {
            let (Some(gid), Some(tp_b), Some(_ver), Some(new_id)) =
                (st.read_u32(), st.read_str_bytes(), st.read_u32(), st.read_u32())
            else {
                return;
            };
            let tp = String::from_utf8_lossy(tp_b).into_owned();
            if tp == "PipeWire:Interface:Factory" && (1..=4).contains(&gid) {
                self.clients[ci].alloc_proxy(new_id, gid, PROXY_KIND_FACTORY);
                let (fn_s, ft_s, fv, fd) = build_factory_dict(gid);
                let mut b = PodBuilder::new(1024);
                b.push_struct();
                b.int(gid as i32);
                b.str_val(Some(fn_s));
                b.str_val(Some(ft_s));
                b.int(fv as i32);
                b.long(1);
                b.dict(&fd);
                b.pop();
                let pay = b.into_vec();
                let _ = self.clients[ci].send_msg(new_id, PW_FACTORY_E_INFO, &pay, &[]);
            } else if tp == "PipeWire:Interface:Metadata" {
                self.clients[ci].alloc_proxy(new_id, gid, PROXY_KIND_METADATA);
                if gid == 5 {
                    let metas = self.metas.clone();
                    for e in metas {
                        let s: u32 = e.subject.parse().unwrap_or(0);
                        let mut b = PodBuilder::new(512);
                        b.push_struct();
                        b.int(s as i32);
                        b.str_val(Some(&e.key));
                        b.str_val(Some(&e.val_type));
                        b.str_val(Some(&e.val));
                        b.pop();
                        let pay = b.into_vec();
                        let _ = self.clients[ci].send_msg(new_id, PW_META_E_PROPERTY, &pay, &[]);
                    }
                }
            } else if tp == "PipeWire:Interface:Client" {
                self.clients[ci].alloc_proxy(new_id, gid, PROXY_KIND_CLIENT);
                if let Some(target) = self.clients.iter().find(|c| c.in_use && c.global_id == gid) {
                    let mut b = PodBuilder::new(1024);
                    b.push_struct();
                    b.int(gid as i32);
                    b.long(1);
                    b.dict(&target.props);
                    b.pop();
                    let pay = b.into_vec();
                    let _ = self.clients[ci].send_msg(new_id, PW_CLIENT_E_INFO, &pay, &[]);
                }
            } else if tp == "PipeWire:Interface:Node" {
                self.clients[ci].alloc_proxy(new_id, gid, PROXY_KIND_NODE);
                if let Some(nidx) = self.find_node_by_global(gid) {
                    self.emit_node_info_for_idx(ci, new_id, nidx);
                }
            } else if tp == "PipeWire:Interface:Port" {
                self.clients[ci].alloc_proxy(new_id, gid, PROXY_KIND_PORT);
                if let Some(nidx) = self
                    .nodes
                    .iter()
                    .position(|n| n.in_use && n.port_registered && n.port_global_id == gid)
                {
                    self.emit_port_info_for_idx(ci, new_id, nidx);
                }
            } else if tp == "PipeWire:Interface:Link" {
                self.clients[ci].alloc_proxy(new_id, gid, PROXY_KIND_LINK);
                if let Some(li) = self
                    .links
                    .iter()
                    .position(|l| l.in_use && l.global_id == gid)
                {
                    self.emit_link_info_to(ci, new_id, li);
                }
            } else {
                self.clients[ci].alloc_proxy(new_id, gid, PROXY_KIND_CLIENT);
            }
        } else if op == PW_REG_M_DESTROY {
            if let Some(gid) = st.read_u32() {
                for li in 0..PWB_MAX_LINKS {
                    if self.links[li].in_use && self.links[li].global_id == gid {
                        self.destroy_link(li);
                    }
                }
            }
        }
    }

    fn handle_client_obj_msg(&mut self, ci: usize, px: PwProxy, op: u8, payload: &[u8]) {
        let Some(mut cur) = PodCursor::new(payload, 0, payload.len()) else {
            return;
        };
        let Some(mut st) = cur.enter_struct_cursor() else {
            return;
        };
        match op {
            PW_CLIENT_M_UPDATE_PROPS => {
                let mut props = self.clients[ci].props.clone();
                if st.read_dict(&mut props, true).is_some() {
                    if let Some(s) = props.get_str("application.name").filter(|s| !s.is_empty()) {
                        self.clients[ci].app_name = s;
                    }
                    if let Some(s) = props
                        .get_str("application.process.binary")
                        .filter(|s| !s.is_empty())
                    {
                        self.clients[ci].binary = s;
                    }
                    if let Some(s) = props
                        .get_str("pipewire.access.portal.app_id")
                        .filter(|s| !s.is_empty())
                    {
                        self.clients[ci].portal_app_id = s;
                    }
                    if props
                        .get_str("pipewire.access.portal.is_portal")
                        .as_deref()
                        == Some("true")
                    {
                        self.clients[ci].is_portal = true;
                    }
                    self.clients[ci].props = props;
                    let mut b = PodBuilder::new(2048);
                    b.push_struct();
                    b.int(self.clients[ci].global_id as i32);
                    b.long(1);
                    b.dict(&self.clients[ci].props);
                    b.pop();
                    let pay = b.into_vec();
                    let _ = self.clients[ci].send_msg(1, PW_CLIENT_E_INFO, &pay, &[]);
                }
            }
            PW_CLIENT_M_GET_PERMS => {
                let mut b = PodBuilder::new(128);
                b.push_struct();
                b.int(0);
                b.push_struct();
                b.int(1);
                b.int(-1);
                b.int(PWB_PERM_ALL as i32);
                b.pop();
                b.pop();
                let pay = b.into_vec();
                let _ = self.clients[ci].send_msg(px.proxy_id, PW_CLIENT_E_PERMS, &pay, &[]);
            }
            PW_CLIENT_M_UPDATE_PERMS => {}
            _ => {}
        }
    }

    fn handle_node_or_port_enum(
        &mut self,
        ci: usize,
        px: PwProxy,
        op: u8,
        req_seq: u32,
        payload: &[u8],
        is_port: bool,
    ) {
        let Some(mut cur) = PodCursor::new(payload, 0, payload.len()) else {
            return;
        };
        let Some(mut st) = cur.enter_struct_cursor() else {
            return;
        };
        let nidx = if is_port {
            self.nodes
                .iter()
                .position(|n| n.in_use && n.port_registered && n.port_global_id == px.global_id)
        } else {
            self.find_node_by_global(px.global_id)
        };
        let Some(nidx) = nidx else {
            return;
        };

        let expected_op = if is_port {
            PW_PORT_M_ENUM_PARAMS
        } else {
            PW_NODE_M_ENUM_PARAMS
        };
        if op != expected_op {
            return;
        }
        let (Some(eseq), Some(pid)) = (st.read_int(), st.read_u32()) else {
            return;
        };
        let index = st.read_u32().unwrap_or(0);
        let num = st.read_u32().unwrap_or(100);

        let use_port_params = is_port
            || (!self.nodes[nidx].port_params.is_empty()
                && (self.nodes[nidx].node_params.is_empty()
                    || (pid != 0 && !self.nodes[nidx].node_params.iter().any(|p| p.id == pid))));
        let arr: &[RawParam] = if use_port_params {
            &self.nodes[nidx].port_params
        } else {
            &self.nodes[nidx].node_params
        };
        let ev_param = if is_port {
            PW_PORT_E_PARAM
        } else {
            PW_NODE_E_PARAM
        };

        let mut msgs = Vec::new();
        let mut found_idx = 0u32;
        for p in arr {
            if pid == 0 || p.id == pid {
                if found_idx >= index && found_idx < index.saturating_add(num) {
                    let mut b = PodBuilder::new(2048);
                    b.push_struct();
                    b.int(eseq);
                    b.id(p.id);
                    b.int(found_idx as i32);
                    b.int((found_idx + 1) as i32);
                    b.raw_pod(&p.pod);
                    b.pop();
                    msgs.push(b.into_vec());
                }
                found_idx += 1;
            }
        }
        for m in msgs {
            let _ = self.clients[ci].send_msg(px.proxy_id, ev_param, &m, &[]);
        }

        let mut dn = PodBuilder::new(64);
        dn.push_struct();
        dn.int(px.proxy_id as i32);
        dn.int((req_seq | SPA_ASYNC_BIT) as i32);
        dn.pop();
        let dn_pay = dn.into_vec();
        let _ = self.clients[ci].send_msg(0, PW_CORE_E_DONE, &dn_pay, &[]);
    }

    fn handle_client_node_msg(
        &mut self,
        ci: usize,
        px: PwProxy,
        op: u8,
        payload: &[u8],
        msg_fds: &[OwnedFd],
    ) {
        let Some(nidx) = self.find_node_by_global(px.global_id) else {
            return;
        };
        let Some(mut cur) = PodCursor::new(payload, 0, payload.len()) else {
            return;
        };
        let Some(mut st) = cur.enter_struct_cursor() else {
            return;
        };

        match op {
            PW_CN_M_GET_NODE => {
                let (Some(_ver), Some(new_id)) = (st.read_u32(), st.read_u32()) else {
                    return;
                };
                self.nodes[nidx].node_proxy_id = new_id;
                let ngid = self.nodes[nidx].global_id;
                self.clients[ci].alloc_proxy(new_id, ngid, PROXY_KIND_NODE);
                self.emit_node_info_for_idx(ci, new_id, nidx);
            }
            PW_CN_M_UPDATE => {
                let (Some(cm), Some(np)) = (st.read_u32(), st.read_int()) else {
                    return;
                };
                if !(0..=64).contains(&np) {
                    return;
                }
                if (cm & 1) != 0 {
                    self.nodes[nidx].node_params.clear();
                }
                for _ in 0..np {
                    let Some((psz, ptp, pbody, ptot)) = st.next_pod() else {
                        return;
                    };
                    if ptp == SPA_OBJECT && psz >= 8 && pbody >= 8 {
                        let pod_start = pbody - 8;
                        let pod_end = pod_start.saturating_add(ptot).min(payload.len());
                        let oid = u32::from_le_bytes([
                            payload[pbody + 4],
                            payload[pbody + 5],
                            payload[pbody + 6],
                            payload[pbody + 7],
                        ]);
                        if self.nodes[nidx].node_params.len() < PWB_MAX_PARAMS {
                            self.nodes[nidx].node_params.push(RawParam {
                                id: oid,
                                pod: payload[pod_start..pod_end].to_vec(),
                            });
                        }
                    }
                }
                if let Some((isz, itp, ibody, _)) = st.next_pod() {
                    if itp == SPA_STRUCT {
                        if let Some(mut inf) = PodCursor::new(payload, ibody, ibody + isz as usize) {
                            if let (Some(max_in), Some(max_out), Some(ich), Some(_ifl)) =
                                (inf.read_int(), inf.read_int(), inf.read_long(), inf.read_long())
                            {
                                if max_in > 0 && max_out == 0 {
                                    self.nodes[nidx].direction = PWB_DIR_IN;
                                } else if max_out > 0 && max_in == 0 {
                                    self.nodes[nidx].direction = PWB_DIR_OUT;
                                }
                                if (ich & 16) != 0 {
                                    let _ = inf.read_dict(&mut self.nodes[nidx].node_props, false);
                                    self.nodes[nidx].apply_target_props();
                                }
                            }
                        }
                    }
                }
                self.broadcast_node_updates(nidx);
                self.try_autolink_all();
            }
            PW_CN_M_PORT_UPDATE => {
                let (Some(dir), Some(_pid), Some(cm), Some(np)) =
                    (st.read_u32(), st.read_u32(), st.read_u32(), st.read_int())
                else {
                    return;
                };
                if !(0..=64).contains(&np) {
                    return;
                }
                self.nodes[nidx].direction = dir as u8;
                if (cm & 1) != 0 {
                    self.nodes[nidx].port_params.clear();
                    self.nodes[nidx].vfmt_candidates.clear();
                }
                for _ in 0..np {
                    let Some((psz, ptp, pbody, ptot)) = st.next_pod() else {
                        return;
                    };
                    if ptp == SPA_OBJECT && psz >= 8 && pbody >= 8 {
                        let pod_start = pbody - 8;
                        let pod_end = pod_start.saturating_add(ptot).min(payload.len());
                        let oid = u32::from_le_bytes([
                            payload[pbody + 4],
                            payload[pbody + 5],
                            payload[pbody + 6],
                            payload[pbody + 7],
                        ]);
                        let raw = &payload[pod_start..pod_end];
                        if self.nodes[nidx].port_params.len() < PWB_MAX_PARAMS {
                            self.nodes[nidx].port_params.push(RawParam {
                                id: oid,
                                pod: raw.to_vec(),
                            });
                        }
                        let parsed = parse_param_pod(raw);
                        if parsed.kind == 1
                            && parsed.vfmt.valid
                            && self.nodes[nidx].vfmt_candidates.len() < PWB_MAX_VFMT
                        {
                            self.nodes[nidx].vfmt_candidates.push(parsed.vfmt);
                        } else if parsed.kind == 2 && parsed.bcaps.valid {
                            self.nodes[nidx].bcaps = parsed.bcaps;
                        }
                    }
                }
                if let Some((isz, itp, ibody, _)) = st.next_pod() {
                    if itp == SPA_STRUCT {
                        if let Some(mut inf) = PodCursor::new(payload, ibody, ibody + isz as usize) {
                            if let (Some(ich), Some(ifl), Some(_), Some(_)) =
                                (inf.read_long(), inf.read_long(), inf.read_int(), inf.read_int())
                            {
                                if (ifl & (1 << 2)) != 0 {
                                    self.nodes[nidx].alloc_buffers = true;
                                }
                                if (ich & 4) != 0 {
                                    let _ = inf.read_dict(&mut self.nodes[nidx].port_props, false);
                                }
                            }
                        }
                    }
                }
                if !self.nodes[nidx].port_registered {
                    let pgid = self.next_global_id;
                    self.next_global_id += 1;
                    self.nodes[nidx].port_registered = true;
                    self.nodes[nidx].port_global_id = pgid;
                    let ngid = self.nodes[nidx].global_id;
                    let dstr = if self.nodes[nidx].direction == PWB_DIR_OUT {
                        "out"
                    } else {
                        "in"
                    };
                    self.nodes[nidx].port_props.set_str("node.id", &ngid.to_string());
                    self.nodes[nidx].port_props.set_str("port.id", "0");
                    self.nodes[nidx].port_props.set_str("port.direction", dstr);
                    let pprops = self.nodes[nidx].port_props.clone();
                    self.broadcast_global(pgid, "PipeWire:Interface:Port", 3, &pprops);
                    self.broadcast_node_updates(nidx);
                }
                if (cm & 1) != 0 && np > 0 {
                    for li in 0..PWB_MAX_LINKS {
                        if !self.links[li].in_use {
                            continue;
                        }
                        if self.links[li].out_node_idx as usize != nidx
                            && self.links[li].in_node_idx as usize != nidx
                        {
                            continue;
                        }
                        let prod_idx = self.links[li].out_node_idx as usize;
                        let cons_idx = self.links[li].in_node_idx as usize;
                        if self.links[li].state == 4 && nidx == prod_idx {
                            let cur_w = self.nodes[prod_idx].neg_w;
                            let cur_h = self.nodes[prod_idx].neg_h;
                            let res_changed = !self.nodes[prod_idx].vfmt_candidates.is_empty()
                                && cur_w > 0
                                && cur_h > 0
                                && !self.nodes[prod_idx]
                                    .vfmt_candidates
                                    .iter()
                                    .any(|v| v.w_def == cur_w && v.h_def == cur_h);
                            if !res_changed {
                                continue;
                            }
                            let pc_idx = self.nodes[prod_idx].client_idx as usize;
                            let cc_idx = self.nodes[cons_idx].client_idx as usize;
                            let prod_cn = self.nodes[prod_idx].cn_proxy_id;
                            let cons_cn = self.nodes[cons_idx].cn_proxy_id;
                            self.cn_send_command(cc_idx, cons_cn, SPA_NODE_CMD_PAUSE);
                            self.cn_send_command(pc_idx, prod_cn, SPA_NODE_CMD_PAUSE);
                            self.cn_send_port_use_buffers_clear(cc_idx, cons_idx, 0);
                            self.cn_send_port_use_buffers_clear(pc_idx, prod_idx, 0);
                            self.links[li].state = 1;
                            self.links[li].io_ready = false;
                            self.links[li].buffers_ready = false;
                            self.links[li].mod_fixated = false;
                            self.links[li].awaiting_mod_fixation = false;
                            self.nodes[prod_idx].free_buffers();
                            self.nodes[prod_idx].awaiting_port_buffers = false;
                            if !self.negotiate_and_activate_link(li) {
                                self.destroy_link(li);
                            }
                            continue;
                        }
                        if self.links[li].state >= 2 {
                            continue;
                        }
                        if self.nodes[prod_idx].n_bufs > 0
                            || self.nodes[prod_idx].vfmt_candidates.is_empty()
                        {
                            continue;
                        }
                        if self.links[li].awaiting_mod_fixation {
                            if nidx != prod_idx {
                                continue;
                            }
                            self.links[li].awaiting_mod_fixation = false;
                            self.links[li].mod_fixated = true;
                        }
                        self.nodes[prod_idx].awaiting_port_buffers = false;
                        if !self.negotiate_and_activate_link(li) {
                            self.destroy_link(li);
                        }
                    }
                }
                self.try_autolink_all();
            }
            PW_CN_M_SET_ACTIVE => {
                if let Some(act) = st.read_int() {
                    self.nodes[nidx].active = act != 0;
                    self.nodes[nidx].state = if act != 0 { 3 } else { 2 };
                    self.broadcast_node_updates(nidx);
                }
            }
            PW_CN_M_EVENT => {
                if let Some((esz, etp, ebody, _)) = st.next_pod() {
                    if etp == SPA_OBJECT && esz >= 8 {
                        let otype = u32::from_le_bytes([
                            payload[ebody],
                            payload[ebody + 1],
                            payload[ebody + 2],
                            payload[ebody + 3],
                        ]);
                        let oid = u32::from_le_bytes([
                            payload[ebody + 4],
                            payload[ebody + 5],
                            payload[ebody + 6],
                            payload[ebody + 7],
                        ]);
                        if otype == EVT_NODE && oid == SPA_NODE_EVT_REQUEST_PROCESS {
                            self.nodes[nidx].stamp_activation_clock(mono_ns());
                            if let Some(amap) = self.nodes[nidx].act_map.as_mut() {
                                let asl = amap.as_mut_slice();
                                if asl.len() >= 20 {
                                    asl[8..12].copy_from_slice(&0i32.to_le_bytes());
                                    asl[12..16].copy_from_slice(&1i32.to_le_bytes());
                                    asl[16..20].copy_from_slice(&0i32.to_le_bytes());
                                    atomic_u32_store(
                                        asl.as_mut_ptr().cast::<u32>(),
                                        1,
                                        Ordering::Release,
                                    );
                                }
                            }
                            if let Some(rfd) = self.nodes[nidx].act_readfd.as_ref() {
                                eventfd_signal(rfd.as_raw_fd());
                            }
                        }
                    }
                }
            }
            PW_CN_M_PORT_BUFFERS => {
                let (Some(_dir), Some(_pid), Some(_mid), Some(mut n_buffers)) =
                    (st.read_u32(), st.read_u32(), st.read_u32(), st.read_u32())
                else {
                    return;
                };
                if (n_buffers as usize) > PWB_MAX_BUFFERS {
                    n_buffers = PWB_MAX_BUFFERS as u32;
                }
                self.nodes[nidx].free_buffers();
                self.nodes[nidx].alloc_buffers = true;
                for _ in 0..n_buffers {
                    let Some(mut n_datas) = st.read_u32() else {
                        break;
                    };
                    if n_datas == 0 {
                        break;
                    }
                    n_datas = n_datas.min(16);
                    for j in 0..n_datas {
                        let (Some(dt), Some(fd_idx), Some(dfl), Some(dmo), Some(dms)) = (
                            st.read_u32(),
                            st.read_long(),
                            st.read_u32(),
                            st.read_u32(),
                            st.read_u32(),
                        ) else {
                            break;
                        };
                        if j == 0 && fd_idx >= 0 && dms > 0 && dms <= 128 * 1024 * 1024 {
                            if let Some(src_fd) = msg_fds.get(fd_idx as usize) {
                                if let Some(dup) = dup_fd_cloexec(src_fd.as_raw_fd()) {
                                    self.nodes[nidx].bufs.push(NodeBuf {
                                        fd: Some(dup),
                                        mem_type: dt,
                                        _flags: dfl,
                                        mapoffset: dmo,
                                        maxsize: dms,
                                        mapped: None,
                                    });
                                }
                            }
                        }
                    }
                }
                self.nodes[nidx].n_bufs = self.nodes[nidx].bufs.len() as u32;
                if self.nodes[nidx].awaiting_port_buffers {
                    self.nodes[nidx].awaiting_port_buffers = false;
                    let pli = self.nodes[nidx].pending_link_idx as usize;
                    if pli < PWB_MAX_LINKS && self.links[pli].in_use {
                        self.complete_link_with_prod_buffers(pli);
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_metadata_msg(&mut self, px: PwProxy, op: u8, payload: &[u8]) {
        if op == PW_META_M_CLEAR {
            if px.global_id == 5 {
                self.metas.clear();
            }
            self.broadcast_metadata_prop(px.global_id, 0, None, None, None);
            return;
        }
        if op != PW_META_M_SET_PROPERTY {
            return;
        }
        let Some(mut cur) = PodCursor::new(payload, 0, payload.len()) else {
            return;
        };
        let Some(mut st) = cur.enter_struct_cursor() else {
            return;
        };
        let (Some(subj), Some(kb), Some(tb), Some(vb)) = (
            st.read_u32(),
            st.read_str_bytes(),
            st.read_str_bytes(),
            st.read_str_bytes(),
        ) else {
            return;
        };
        let key = String::from_utf8_lossy(kb).into_owned();
        let tp = String::from_utf8_lossy(tb).into_owned();
        let val = String::from_utf8_lossy(vb).into_owned();

        if px.global_id == 5 {
            let sbuf = subj.to_string();
            if key.is_empty() {
                self.metas.retain(|e| e.subject != sbuf);
            } else if val.is_empty() {
                self.metas.retain(|e| !(e.subject == sbuf && e.key == key));
            } else if let Some(found) = self
                .metas
                .iter_mut()
                .find(|e| e.subject == sbuf && e.key == key)
            {
                found.val_type = tp.clone();
                found.val = val.clone();
            } else if self.metas.len() < 16 {
                self.metas.push(MetaEntry {
                    subject: sbuf,
                    key: key.clone(),
                    val_type: tp.clone(),
                    val: val.clone(),
                });
            }
        }

        self.broadcast_metadata_prop(
            px.global_id,
            subj,
            (!key.is_empty()).then_some(key.as_str()),
            (!tp.is_empty()).then_some(tp.as_str()),
            (!val.is_empty()).then_some(val.as_str()),
        );

        if key == "target.object" || key == "target.node" {
            if let Some(nidx) = self.find_node_by_global(subj) {
                if val.is_empty() || val == "-1" {
                    self.nodes[nidx].target_str.clear();
                    self.nodes[nidx].target_id_req = 0;
                } else {
                    self.nodes[nidx].target_id_req = parse_u32_strict(&val).unwrap_or(0);
                    self.nodes[nidx].target_str = val;
                }
                self.try_autolink_all();
            }
        }
    }

    fn dispatch_client_packet(
        &mut self,
        ci: usize,
        oid: u32,
        op: u8,
        seq: u32,
        payload: &[u8],
        msg_fds: &[OwnedFd],
    ) {
        if oid == 0 {
            self.handle_core_msg(ci, op, payload);
            return;
        }
        let Some(px) = self.clients[ci].find_proxy(oid) else {
            return;
        };
        match px.kind {
            PROXY_KIND_REGISTRY => self.handle_registry_msg(ci, op, payload),
            PROXY_KIND_CLIENT => self.handle_client_obj_msg(ci, px, op, payload),
            PROXY_KIND_CLIENT_NODE => self.handle_client_node_msg(ci, px, op, payload, msg_fds),
            PROXY_KIND_NODE => self.handle_node_or_port_enum(ci, px, op, seq, payload, false),
            PROXY_KIND_PORT => self.handle_node_or_port_enum(ci, px, op, seq, payload, true),
            PROXY_KIND_METADATA => self.handle_metadata_msg(px, op, payload),
            _ => {}
        }
    }

    fn drain_client_socket(&mut self, ci: usize) -> i32 {
        let fd = self.clients[ci].raw_fd();
        if fd < 0 {
            return -1;
        }
        let mut tmp = [0u8; 65536];
        match recvmsg_with_fds(fd, &mut tmp) {
            Ok(RecvFdsResult::Eof) | Err(()) => return -1,
            Ok(RecvFdsResult::WouldBlock) => return 0,
            Ok(RecvFdsResult::Data { bytes, fds }) => {
                let incoming = fds.len() as u32;
                let (quota_ok, keep) = fd_quota_check(self.clients[ci].rx_fds.len() as u32, incoming);
                if !quota_ok {
                    record_trip(
                        FS_FD_QUOTA,
                        b"PW client exceeded SCM_RIGHTS FD quota (256)",
                    );
                    return -1;
                }
                self.clients[ci].rx_fds.extend(fds.into_iter().take(keep as usize));

                let cur_len = self.clients[ci].rx_buf.len();
                if cur_len.saturating_add(bytes) > PWB_MAX_PACKET_SZ as usize {
                    record_trip(
                        FS_PW_BAD_PKT,
                        b"PW client exceeded 16MiB stream buffer limit",
                    );
                    return -1;
                }
                self.clients[ci].rx_buf.extend_from_slice(&tmp[..bytes]);
            }
        }

        let mut consumed = 0usize;
        loop {
            let rem = &self.clients[ci].rx_buf[consumed..];
            if rem.len() < 16 {
                break;
            }
            let hdr = match parse_pkt_hdr_status(rem) {
                PktHdrStatus::Ready(h) => h,
                PktHdrStatus::Incomplete => break,
                PktHdrStatus::Malformed => {
                    record_trip(
                        FS_PW_BAD_PKT,
                        b"PW client sent oversized or malformed packet header",
                    );
                    return -1;
                }
            };
            if rem.len() < hdr.pkt_len {
                break;
            }
            if (self.clients[ci].rx_fds.len() as u32) < hdr.nfds {
                break;
            }
            let local_fds: Vec<OwnedFd> = self.clients[ci]
                .rx_fds
                .drain(..hdr.nfds as usize)
                .collect();
            let pay = self.clients[ci].rx_buf[consumed + 16..consumed + 16 + hdr.psz as usize]
                .to_vec();
            self.dispatch_client_packet(ci, hdr.oid, hdr.op, hdr.seq, &pay, &local_fds);
            drop(local_fds);
            consumed += hdr.pkt_len;
        }
        if consumed > 0 {
            self.clients[ci].rx_buf.drain(..consumed);
        }
        1
    }

    fn accept_from_listener(&mut self, is_mgr: bool) {
        let stream = {
            let l = if is_mgr { &self.mgr } else { &self.listen };
            let Some(listener) = l.as_ref() else {
                return;
            };
            let Ok((s, _)) = listener.accept() else {
                return;
            };
            let _ = s.set_nonblocking(true);
            s
        };
        let Some(free_i) = self.clients.iter().position(|c| !c.in_use) else {
            return;
        };
        let gid = self.next_global_id;
        let ser = self.next_serial;
        self.next_global_id += 1;
        self.next_serial += 1;

        let Some(uc) = socket_peercred(stream.as_raw_fd()) else {
            return;
        };
        let my_uid = current_uid();
        if uc.uid != my_uid && uc.uid != 0 {
            return;
        }
        let pid = uc.pid.max(0) as u32;
        let (app, bin) = detect_proc_info(pid);
        let mut props = Dict::default();
        props.set_str("application.name", &app);
        props.set_str("application.process.binary", &bin);
        props.set_str("pipewire.sec.pid", &pid.to_string());
        props.set_str("pipewire.sec.uid", &uc.uid.to_string());
        props.set_str("pipewire.sec.gid", &uc.gid.to_string());
        props.set_str("object.serial", &ser.to_string());
        props.set_str("module.id", "2");
        props.set_str("pipewire.protocol", "protocol-native");

        self.clients[free_i] = PwClient {
            in_use: true,
            stream: Some(stream),
            global_id: gid,
            pid,
            app_name: app,
            binary: bin,
            props,
            ..PwClient::default()
        };
    }
}

fn build_factory_dict(idx: u32) -> (&'static str, &'static str, u32, Dict) {
    let mut d = Dict::default();
    let (n, t, v) = match idx {
        1 => ("client-node", "PipeWire:Interface:ClientNode", 6u32),
        2 => ("link-factory", "PipeWire:Interface:Link", 3u32),
        3 => ("adapter", "PipeWire:Interface:Node", 3u32),
        _ => ("metadata", "PipeWire:Interface:Metadata", 3u32),
    };
    d.set_str("factory.name", n);
    d.set_str("factory.type.name", t);
    d.set_str("factory.type.version", &v.to_string());
    d.set_str("module.id", "2");
    (n, t, v, d)
}

fn build_metadata_dict(gid: u32) -> Dict {
    let mut d = Dict::default();
    d.set_str("metadata.name", if gid == 5 { "default" } else { "settings" });
    d.set_str("object.serial", if gid == 5 { "5" } else { "6" });
    d
}

pub struct PwBrokerHandle {
    state: Arc<Mutex<PwState>>,
    shared: Arc<SharedAtomics>,
    thread: Option<JoinHandle<()>>,
}

impl PwBrokerHandle {
    fn signal_wake(&self) {
        if let Ok(st) = self.state.lock() {
            if let Some(w) = st.wake_efd.as_ref() {
                eventfd_signal(w.as_raw_fd());
            }
        }
    }
}

fn broker_thread_main(state: Arc<Mutex<PwState>>, shared: Arc<SharedAtomics>) {
    set_thread_nice(-10);
    let mut pfds: Vec<PollFd> = Vec::with_capacity(128);
    let mut meta: Vec<(u8, usize)> = Vec::with_capacity(128); // 0=wake, 1=listen, 2=mgr, 3=client, 4=prod_done, 5=cons_done

    while shared.running.load(Ordering::Relaxed) {
        pfds.clear();
        meta.clear();
        {
            let st = state.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(w) = st.wake_efd.as_ref() {
                pfds.push(PollFd {
                    fd: w.as_raw_fd(),
                    events: POLLIN,
                    revents: 0,
                });
                meta.push((0, 0));
            }
            if let Some(l) = st.listen.as_ref() {
                pfds.push(PollFd {
                    fd: l.as_raw_fd(),
                    events: POLLIN,
                    revents: 0,
                });
                meta.push((1, 0));
            }
            if let Some(m) = st.mgr.as_ref() {
                pfds.push(PollFd {
                    fd: m.as_raw_fd(),
                    events: POLLIN,
                    revents: 0,
                });
                meta.push((2, 0));
            }
            for (ci, c) in st.clients.iter().enumerate() {
                if c.in_use && c.raw_fd() >= 0 {
                    pfds.push(PollFd {
                        fd: c.raw_fd(),
                        events: POLLIN | POLLHUP | POLLERR,
                        revents: 0,
                    });
                    meta.push((3, ci));
                }
            }
            for (ni, n) in st.nodes.iter().enumerate() {
                if n.in_use {
                    if let Some(defd) = n.done_efd.as_ref() {
                        pfds.push(PollFd {
                            fd: defd.as_raw_fd(),
                            events: POLLIN,
                            revents: 0,
                        });
                        meta.push((if n.direction == PWB_DIR_OUT { 4 } else { 5 }, ni));
                    }
                    if n.direction != PWB_DIR_OUT {
                        if let Some(awfd) = n.act_writefd.as_ref() {
                            pfds.push(PollFd {
                                fd: awfd.as_raw_fd(),
                                events: POLLIN,
                                revents: 0,
                            });
                            meta.push((5, ni));
                        }
                    }
                }
            }
        }

        if do_poll(&mut pfds, 250) <= 0 {
            continue;
        }

        let mut st = state.lock().unwrap_or_else(|p| p.into_inner());
        for (pfd, &(kind, idx)) in pfds.iter().zip(meta.iter()) {
            if pfd.revents == 0 {
                continue;
            }
            match kind {
                0 => {
                    if let Some(w) = st.wake_efd.as_ref() {
                        eventfd_drain(w.as_raw_fd());
                    }
                }
                1 => st.accept_from_listener(false),
                2 => st.accept_from_listener(true),
                3 => {
                    if st.clients.get(idx).is_some_and(|c| c.in_use)
                        && ((pfd.revents & (POLLERR | POLLHUP)) != 0
                            || st.drain_client_socket(idx) < 0)
                    {
                        st.handle_client_disconnect(idx);
                    }
                }
                4 => st.on_prod_done_event(idx),
                5 => st.on_cons_done_event(idx),
                _ => {}
            }
        }
    }
}

fn preview_shm_name_string() -> String {
    if let Some(dir) = ensure_private_dir() {
        let cand = format!("{}/pw_preview", dir);
        if cand.len() < 64 {
            return cand;
        }
    }
    format!("/lwasapi_pw_preview_{}", current_uid())
}

#[no_mangle]
pub unsafe extern "C" fn lwasapi_pw_preview_shm_name(out_buf: *mut c_char, out_len: usize) {
    if out_buf.is_null() || out_len == 0 {
        return;
    }
    let sl = unsafe { core::slice::from_raw_parts_mut(out_buf.cast::<u8>(), out_len) };
    copy_str_to_c(sl, &preview_shm_name_string());
}

#[no_mangle]
pub unsafe extern "C" fn lwasapi_pw_broker_start(
    out_brk: *mut *mut PwBrokerHandle,
    opts: *const LwasapiPwBrokerOpts,
) -> c_int {
    if out_brk.is_null() {
        return -22; // -EINVAL
    }
    let shared = Arc::new(SharedAtomics::default());
    let mut st = PwState::new(Arc::clone(&shared));
    st.wake_efd = new_eventfd();

    let opt_rdir = (!opts.is_null())
        .then(|| unsafe { ptr_to_opt_str((*opts).runtime_dir) })
        .flatten();
    let opt_sname = (!opts.is_null())
        .then(|| unsafe { ptr_to_opt_str((*opts).socket_name) })
        .flatten();
    let opt_ign = (!opts.is_null()) && unsafe { (*opts).ignore_zero_flag };

    let rdir = opt_rdir
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("LWASAPI_PW_TEST_DIR").ok().filter(|s| !s.is_empty()))
        .or_else(|| std::env::var("XDG_RUNTIME_DIR").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| format!("/run/user/{}", current_uid()));
    let sname = opt_sname
        .filter(|s| !s.is_empty() && !s.contains('/') && *s != "." && *s != "..")
        .or_else(|| {
            std::env::var("LWASAPI_PW_SOCKET_NAME")
                .ok()
                .filter(|s| !s.is_empty() && !s.contains('/') && s != "." && s != "..")
        })
        .unwrap_or_else(|| "pipewire-0".to_string());

    st.runtime_dir = rdir;
    st.socket_path = format!("{}/{}", st.runtime_dir, sname);
    st.manager_socket_path = format!("{}/{}-manager", st.runtime_dir, sname);
    st.ignore_zero_flag = opt_ign || std::env::var_os("LWASAPI_PW_SOCKET_NAME").is_some();

    let isolated = st.ignore_zero_flag || std::env::var_os("LWASAPI_PW_TEST_DIR").is_some();
    let shm_name = if isolated {
        format!("{}/pw_preview_test", st.runtime_dir)
    } else {
        unlink_shm_path(&format!("/lwasapi_pw_preview_{}", current_uid()), true);
        preview_shm_name_string()
    };
    st.preview_shm_name = shm_name.clone();
    if let Some((fd, is_posix)) = open_private_shm(&shm_name, size_of::<LwasapiPwPreviewShm>()) {
        st.preview_shm_posix = is_posix;
        if let Some(mut map) =
            MappedMem::map_fd(fd.as_raw_fd(), size_of::<LwasapiPwPreviewShm>(), true, 0)
        {
            let hdr_end = offset_of!(LwasapiPwPreviewShm, pixels);
            map.as_mut_slice()[..hdr_end].fill(0);
            let shm = map.as_mut_ptr().cast::<LwasapiPwPreviewShm>();
            unsafe {
                (*shm).magic = LWASAPI_PW_PREVIEW_MAGIC;
                (*shm).version = LWASAPI_PW_PREVIEW_VERSION;
            }
            st.preview_shm_map = Some(map);
        }
        st.preview_shm_fd = Some(fd);
    }

    let _ = st.apply_bind_mode();
    shared.running.store(true, Ordering::Release);

    let state = Arc::new(Mutex::new(st));
    let thread_state = Arc::clone(&state);
    let thread_shared = Arc::clone(&shared);
    let handle = match thread::Builder::new()
        .name("salsa-pw-broker".into())
        .spawn(move || broker_thread_main(thread_state, thread_shared))
    {
        Ok(t) => t,
        Err(_) => {
            shared.running.store(false, Ordering::Release);
            return -12;
        }
    };

    let boxed = Box::new(PwBrokerHandle {
        state,
        shared,
        thread: Some(handle),
    });
    unsafe {
        *out_brk = Box::into_raw(boxed);
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn lwasapi_pw_broker_stop(brk: *mut PwBrokerHandle) {
    if brk.is_null() {
        return;
    }
    let mut handle = unsafe { Box::from_raw(brk) };
    handle.shared.running.store(false, Ordering::Release);
    handle.signal_wake();
    if let Some(t) = handle.thread.take() {
        let _ = t.join();
    }
    if let Ok(mut st) = handle.state.lock() {
        for ci in 0..PWB_MAX_CLIENTS {
            if st.clients[ci].in_use {
                st.handle_client_disconnect(ci);
            }
        }
        if st.listen.take().is_some() {
            let _ = fs::remove_file(&st.socket_path);
        }
        if st.mgr.take().is_some() {
            let _ = fs::remove_file(&st.manager_socket_path);
        }
        st.preview_shm_map = None;
        st.preview_shm_fd = None;
        unlink_shm_path(&st.preview_shm_name, st.preview_shm_posix);
    };
}

#[no_mangle]
pub unsafe extern "C" fn lwasapi_pw_broker_sync_mode(brk: *mut PwBrokerHandle) -> c_int {
    if brk.is_null() {
        return -22;
    }
    let h = unsafe { &*brk };
    let rc = {
        let mut st = h.state.lock().unwrap_or_else(|p| p.into_inner());
        st.apply_bind_mode()
    };
    h.signal_wake();
    rc
}

#[no_mangle]
pub unsafe extern "C" fn lwasapi_pw_broker_set_preview_ex(
    brk: *mut PwBrokerHandle,
    enabled: bool,
    ttl_ms: u32,
    max_w: u32,
    max_h: u32,
    target_fps: u32,
) {
    if brk.is_null() {
        return;
    }
    let h = unsafe { &*brk };
    let cw = max_w.min(LWASAPI_PW_PREVIEW_MAX_W);
    let ch = max_h.min(LWASAPI_PW_PREVIEW_MAX_H);
    let cfps = target_fps.min(240);
    h.shared.preview_target_w.store(cw, Ordering::Relaxed);
    h.shared.preview_target_h.store(ch, Ordering::Relaxed);
    h.shared.preview_target_fps.store(cfps, Ordering::Relaxed);

    let st = h.state.lock().unwrap_or_else(|p| p.into_inner());
    let Some(map) = st.preview_shm_map.as_ref() else {
        return;
    };
    let shm_ptr = map.as_ptr().cast::<LwasapiPwPreviewShm>().cast_mut();
    let now = mono_ns();
    if enabled {
        let ttl_ns = (if ttl_ms != 0 { ttl_ms } else { 15000 }) as u64 * 1_000_000;
        h.shared
            .preview_expiry_ns
            .store(now.saturating_add(ttl_ns), Ordering::Relaxed);
        unsafe {
            atomic_u32_store(ptr::addr_of_mut!((*shm_ptr).target_max_w), cw, Ordering::Relaxed);
            atomic_u32_store(ptr::addr_of_mut!((*shm_ptr).target_max_h), ch, Ordering::Relaxed);
            atomic_u32_store(ptr::addr_of_mut!((*shm_ptr).target_fps), cfps, Ordering::Relaxed);
            atomic_u64_store(
                ptr::addr_of_mut!((*shm_ptr).last_gui_ping_ns),
                now,
                Ordering::Relaxed,
            );
            atomic_u32_store(
                ptr::addr_of_mut!((*shm_ptr).preview_requested),
                1,
                Ordering::Release,
            );
        }
    } else {
        h.shared.preview_expiry_ns.store(0, Ordering::Relaxed);
        unsafe {
            atomic_u32_store(
                ptr::addr_of_mut!((*shm_ptr).preview_requested),
                0,
                Ordering::Release,
            );
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn lwasapi_pw_broker_set_preview(
    brk: *mut PwBrokerHandle,
    enabled: bool,
    ttl_ms: u32,
) {
    unsafe { lwasapi_pw_broker_set_preview_ex(brk, enabled, ttl_ms, 0, 0, 0) }
}

#[no_mangle]
pub unsafe extern "C" fn lwasapi_pw_broker_set_cast_limits(
    brk: *mut PwBrokerHandle,
    max_w: u32,
    max_h: u32,
    max_fps: u32,
) {
    if brk.is_null() {
        return;
    }
    let h = unsafe { &*brk };
    h.shared.cast_limit_w.store(max_w, Ordering::Relaxed);
    h.shared.cast_limit_h.store(max_h, Ordering::Relaxed);
    h.shared
        .cast_limit_fps
        .store(max_fps.min(240), Ordering::Relaxed);
}

#[no_mangle]
pub unsafe extern "C" fn lwasapi_pw_broker_fill_debug(
    brk: *mut PwBrokerHandle,
    out_ptr: *mut LwasapiDbgPwSnapshot,
) {
    if out_ptr.is_null() {
        return;
    }
    let out = unsafe { &mut *out_ptr };
    *out = unsafe { mem::zeroed() };
    if brk.is_null() {
        return;
    }
    let h = unsafe { &*brk };
    let st = h.state.lock().unwrap_or_else(|p| p.into_inner());
    out.broker_running = u8::from(h.shared.broker_listening.load(Ordering::Relaxed));
    out.zero_pw_mode = u8::from(h.shared.zero_pw_mode.load(Ordering::Relaxed));
    out.preview_requested = st
        .preview_shm_map
        .as_ref()
        .map_or(0, |m| {
            let shm = m.as_ptr().cast::<LwasapiPwPreviewShm>();
            u8::from(unsafe {
                atomic_u32_load(ptr::addr_of!((*shm).preview_requested), Ordering::Relaxed) != 0
            })
        });
    copy_str_to_c(&mut out.socket_path, &st.socket_path);
    copy_str_to_c(&mut out.manager_socket_path, &st.manager_socket_path);
    copy_str_to_c(&mut out.preview_shm_name, &st.preview_shm_name);
    out.frames_relayed_total = h.shared.frames_relayed_total.load(Ordering::Relaxed);
    out.frames_dropped = h.shared.frames_dropped.load(Ordering::Relaxed);
    out.audio_rejects = h.shared.audio_rejects.load(Ordering::Relaxed);
    out.preview_target_w = h.shared.preview_target_w.load(Ordering::Relaxed);
    out.preview_target_h = h.shared.preview_target_h.load(Ordering::Relaxed);
    out.preview_target_fps = h.shared.preview_target_fps.load(Ordering::Relaxed);
    out.cast_limit_w = h.shared.cast_limit_w.load(Ordering::Relaxed);
    out.cast_limit_h = h.shared.cast_limit_h.load(Ordering::Relaxed);
    out.cast_limit_fps = h.shared.cast_limit_fps.load(Ordering::Relaxed);
    out.actual_fps_x100 = h.shared.preview_actual_fps_x100_pub.load(Ordering::Relaxed);

    for (ci, c) in st.clients.iter().enumerate() {
        if (out.num_clients as usize) >= LWASAPI_DEBUG_MAX_PW_CLIENTS {
            break;
        }
        if !c.in_use {
            continue;
        }
        let dc: &mut LwasapiDbgPwClient = &mut out.clients[out.num_clients as usize];
        out.num_clients += 1;
        dc.id = c.global_id;
        dc.pid = c.pid;
        copy_str_to_c(&mut dc.app_name, &c.app_name);
        copy_str_to_c(&mut dc.binary, &c.binary);
        copy_str_to_c(&mut dc.portal_app_id, &c.portal_app_id);
        dc.is_portal = u8::from(c.is_portal);
        dc.num_nodes = st
            .nodes
            .iter()
            .filter(|n| n.in_use && n.client_idx as usize == ci)
            .count() as u32;
    }

    for nd in &st.nodes {
        if (out.num_nodes as usize) >= LWASAPI_DEBUG_MAX_PW_NODES {
            break;
        }
        if !nd.in_use {
            continue;
        }
        let dn: &mut LwasapiDbgPwNode = &mut out.nodes[out.num_nodes as usize];
        out.num_nodes += 1;
        dn.id = nd.global_id;
        let ow = &st.clients[nd.client_idx as usize];
        dn.client_id = if ow.in_use { ow.global_id } else { 0 };
        dn.client_pid = if ow.in_use { ow.pid } else { 0 };
        copy_str_to_c(&mut dn.name, &nd.name);
        copy_str_to_c(&mut dn.media_class, &nd.media_class);
        copy_str_to_c(
            &mut dn.format_name,
            if nd.neg_format != 0 {
                vfmt_name(nd.neg_format)
            } else {
                "negotiating"
            },
        );
        copy_str_to_c(
            &mut dn.mem_type,
            if nd.neg_w != 0 {
                if nd.neg_is_dmabuf {
                    "DmaBuf"
                } else {
                    "MemFd"
                }
            } else {
                "idle"
            },
        );
        dn.direction = nd.direction;
        dn.state = nd.state;
        dn.is_driver = u8::from(nd.is_driver);
        dn.alloc_buffers = u8::from(nd.alloc_buffers);
        dn.width = nd.neg_w;
        dn.height = nd.neg_h;
        dn.fps_num = nd.neg_fps_num;
        dn.fps_denom = nd.neg_fps_denom;
        dn.n_buffers = nd.n_bufs;
        dn.drm_modifier = nd.neg_modifier;
        dn.frames = nd.frames_count;
        dn.target_node_id = nd.target_id_req;
        if nd.neg_w != 0 && nd.direction == PWB_DIR_OUT {
            if nd.neg_is_dmabuf {
                out.dmabuf_streams += 1;
            } else {
                out.memfd_streams += 1;
            }
        }
    }

    for lk in &st.links {
        if (out.num_links as usize) >= LWASAPI_DEBUG_MAX_PW_LINKS {
            break;
        }
        if !lk.in_use {
            continue;
        }
        let dl: &mut LwasapiDbgPwLink = &mut out.links[out.num_links as usize];
        out.num_links += 1;
        let on = &st.nodes[lk.out_node_idx as usize];
        let ind = &st.nodes[lk.in_node_idx as usize];
        dl.id = lk.global_id;
        dl.out_node_id = on.global_id;
        dl.in_node_id = ind.global_id;
        dl.state = lk.state;
        dl.is_dmabuf = u8::from(on.neg_is_dmabuf);
        dl.frames_relayed = lk.frames_relayed;
        if lk.state == 4 {
            out.active_casting = 1;
        }
        let fps = on.neg_fps_num.checked_div(on.neg_fps_denom).unwrap_or(60);
        let sum = format!(
            "{}x{}@{} {} ({})",
            on.neg_w,
            on.neg_h,
            fps,
            vfmt_name(on.neg_format),
            if on.neg_is_dmabuf { "DmaBuf" } else { "MemFd" }
        );
        copy_str_to_c(&mut dl.format_summary, &sum);
    }

    if out.active_casting != 0
        && st.relay_actual_fps_x100 > 0
        && mono_ns().saturating_sub(st.relay_last_frame_ns) <= 2_000_000_000
    {
        out.actual_fps_x100 = st.relay_actual_fps_x100;
    } else if out.preview_requested == 0 {
        out.actual_fps_x100 = 0;
    }
}

#[no_mangle]
pub unsafe extern "C" fn lwasapi_pw_broker_self_test(
    brk: *mut PwBrokerHandle,
    out_msg: *mut c_char,
    out_len: usize,
) -> c_int {
    let write_msg = |msg: &str| {
        if !out_msg.is_null() && out_len > 0 {
            let sl = unsafe { core::slice::from_raw_parts_mut(out_msg.cast::<u8>(), out_len) };
            copy_str_to_c(sl, msg);
        }
    };
    if brk.is_null() {
        write_msg("Video broker is null.");
        return -1;
    }
    let h = unsafe { &*brk };
    if !h.shared.broker_listening.load(Ordering::Relaxed) {
        write_msg("Video broker socket is not listening (Zero-PipeWire mode inactive).");
        return -1;
    }
    let sock_path = h.state.lock().unwrap_or_else(|p| p.into_inner()).socket_path.clone();
    let Ok(stream) = UnixStream::connect(&sock_path) else {
        write_msg(&format!("connect({}) failed", sock_path));
        return -1;
    };
    let _ = stream.set_nonblocking(true);

    let send_pkt = |id: u32, op: u8, seq: u32, build_fn: fn(&mut PodBuilder)| -> bool {
        let mut b = PodBuilder::new(64);
        build_fn(&mut b);
        let pkt = frame_pw_msg(id, op, seq, 0, &b.into_vec());
        sendmsg_all_with_fds(stream.as_raw_fd(), &pkt, &[]).is_ok()
    };
    if !send_pkt(0, PW_CORE_M_HELLO, 0, |b| {
        b.push_struct();
        b.int(4);
        b.pop();
    }) || !send_pkt(0, PW_CORE_M_GET_REGISTRY, 1, |b| {
        b.push_struct();
        b.int(3);
        b.int(2);
        b.pop();
    }) || !send_pkt(0, PW_CORE_M_SYNC, 2, |b| {
        b.push_struct();
        b.int(0);
        b.int(77);
        b.pop();
    }) {
        write_msg(&format!("send failed on {}", sock_path));
        return -1;
    }

    let mut rx = Vec::with_capacity(4096);
    let mut got_info = false;
    let mut got_cn_factory = false;
    let mut got_done = false;
    let t0 = mono_ns();
    while !got_done && mono_ns().saturating_sub(t0) < 1_000_000_000 {
        let mut pfd = [PollFd {
            fd: stream.as_raw_fd(),
            events: POLLIN,
            revents: 0,
        }];
        if do_poll(&mut pfd, 100) <= 0 {
            continue;
        }
        let mut tmp = [0u8; 4096];
        match recvmsg_with_fds(stream.as_raw_fd(), &mut tmp) {
            Ok(RecvFdsResult::Data { bytes, .. }) => rx.extend_from_slice(&tmp[..bytes]),
            Ok(RecvFdsResult::WouldBlock) => continue,
            _ => break,
        }
        let mut off = 0usize;
        while rx.len() - off >= 16 {
            let Some(hdr) = parse_pkt_hdr(&rx[off..]) else {
                break;
            };
            if rx.len() - off < hdr.pkt_len {
                break;
            }
            if hdr.oid == 0 && hdr.op == PW_CORE_E_INFO {
                got_info = true;
            }
            if hdr.oid == 2 && hdr.op == PW_REG_E_GLOBAL {
                let pay = &rx[off + 16..off + 16 + hdr.psz as usize];
                if pay
                    .windows(b"PipeWire:Interface:ClientNode".len())
                    .any(|w| w == b"PipeWire:Interface:ClientNode")
                {
                    got_cn_factory = true;
                }
            }
            if hdr.oid == 0 && hdr.op == PW_CORE_E_DONE {
                got_done = true;
            }
            off += hdr.pkt_len;
        }
        if off > 0 {
            rx.drain(..off);
        }
    }

    if got_info && got_cn_factory && got_done {
        write_msg(&format!(
            "PASS: pipewire-0 native handshake + ClientNode factory + Sync OK ({})",
            sock_path
        ));
        0
    } else {
        write_msg(&format!(
            "FAIL: info={} factory={} done={} on {}",
            u8::from(got_info),
            u8::from(got_cn_factory),
            u8::from(got_done),
            sock_path
        ));
        -1
    }
}

#[no_mangle]
pub unsafe extern "C" fn lwasapi_pw_broker_reset_clients(brk: *mut PwBrokerHandle) -> c_int {
    if brk.is_null() {
        return 0;
    }
    let h = unsafe { &*brk };
    let mut flushed = 0i32;
    {
        let mut st = h.state.lock().unwrap_or_else(|p| p.into_inner());
        for ci in 0..PWB_MAX_CLIENTS {
            if st.clients[ci].in_use {
                st.handle_client_disconnect(ci);
                flushed += 1;
            }
        }
    }
    h.signal_wake();
    flushed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failsafe::{get_stats, reset_stats};
    use crate::pw_wire::{OBJ_FORMAT, VF_BGRX};
    use std::ffi::CString;
    use std::io::{Read, Seek, SeekFrom, Write};

    #[test]
    fn pw_broker_handshake_audio_reject_reset_and_failsafe() {
        reset_stats();
        let tmp_dir = format!("/tmp/salsa_pw_test_{}", mono_ns());
        fs::create_dir_all(&tmp_dir).unwrap();
        let c_rdir = CString::new(tmp_dir.clone()).unwrap();
        let c_sname = CString::new("pipewire-rust-test").unwrap();
        let opts = LwasapiPwBrokerOpts {
            runtime_dir: c_rdir.as_ptr(),
            socket_name: c_sname.as_ptr(),
            ignore_zero_flag: true,
        };
        let mut brk: *mut PwBrokerHandle = ptr::null_mut();
        assert_eq!(unsafe { lwasapi_pw_broker_start(&mut brk, &opts) }, 0);
        assert!(!brk.is_null());

        let mut msg = [0u8; 256];
        let rc = unsafe { lwasapi_pw_broker_self_test(brk, msg.as_mut_ptr().cast(), msg.len()) };
        let txt = String::from_utf8_lossy(&msg);
        assert_eq!(rc, 0, "self_test msg={}", txt);

        // Connect an audio ClientNode (should be rejected by Video-Only broker)
        let sock_path = format!("{}/pipewire-rust-test", tmp_dir);
        let client = UnixStream::connect(&sock_path).unwrap();
        let _ = client.set_nonblocking(true);

        let mut b = PodBuilder::new(64);
        b.push_struct();
        b.int(4);
        b.pop();
        let p_hello = frame_pw_msg(0, PW_CORE_M_HELLO, 0, 0, &b.into_vec());
        sendmsg_all_with_fds(client.as_raw_fd(), &p_hello, &[]).unwrap();

        let mut props = Dict::default();
        props.set_str("media.type", "Audio");
        props.set_str("media.class", "Stream/Output/Audio");
        let mut bc = PodBuilder::new(256);
        bc.push_struct();
        bc.str_val(Some("client-node"));
        bc.str_val(Some("PipeWire:Interface:ClientNode"));
        bc.int(6);
        bc.dict(&props);
        bc.int(9);
        bc.pop();
        let p_create = frame_pw_msg(0, PW_CORE_M_CREATE_OBJ, 1, 0, &bc.into_vec());
        sendmsg_all_with_fds(client.as_raw_fd(), &p_create, &[]).unwrap();

        // Read until PW_CORE_E_ERROR
        let mut rx = Vec::new();
        let mut got_err = false;
        let t0 = mono_ns();
        while !got_err && mono_ns().saturating_sub(t0) < 1_000_000_000 {
            let mut pfd = [PollFd {
                fd: client.as_raw_fd(),
                events: POLLIN,
                revents: 0,
            }];
            if do_poll(&mut pfd, 50) <= 0 {
                continue;
            }
            let mut tmp = [0u8; 4096];
            if let Ok(RecvFdsResult::Data { bytes, .. }) =
                recvmsg_with_fds(client.as_raw_fd(), &mut tmp)
            {
                rx.extend_from_slice(&tmp[..bytes]);
                let mut off = 0;
                while rx.len() - off >= 16 {
                    let hdr = parse_pkt_hdr(&rx[off..]).unwrap();
                    if rx.len() - off < hdr.pkt_len {
                        break;
                    }
                    if hdr.oid == 0 && hdr.op == PW_CORE_E_ERROR {
                        got_err = true;
                    }
                    off += hdr.pkt_len;
                }
                rx.drain(..off);
            }
        }
        assert!(got_err, "audio node should receive PW_CORE_E_ERROR");

        // Trigger malformed packet failsafe on bad client
        let bad = UnixStream::connect(&sock_path).unwrap();
        let mut bad_hdr = [0u8; 16];
        bad_hdr[12..16].copy_from_slice(&999u32.to_le_bytes()); // nfds > 256!
        sendmsg_all_with_fds(bad.as_raw_fd(), &bad_hdr, &[]).unwrap();
        thread::sleep(std::time::Duration::from_millis(60));
        assert!(get_stats().pw_quarantined >= 1);

        // Reset remaining healthy client cleanly
        let flushed = unsafe { lwasapi_pw_broker_reset_clients(brk) };
        assert!(flushed >= 1);

        let mut snap: LwasapiDbgPwSnapshot = unsafe { mem::zeroed() };
        unsafe { lwasapi_pw_broker_fill_debug(brk, &mut snap) };
        assert_eq!(snap.broker_running, 1);
        assert!(snap.audio_rejects >= 1);

        unsafe { lwasapi_pw_broker_stop(brk) };
        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn pw_broker_video_relay_and_preview_shm() {
        let tmp_dir = format!("/tmp/salsa_pw_relay_{}", mono_ns());
        fs::create_dir_all(&tmp_dir).unwrap();
        let c_rdir = CString::new(tmp_dir.clone()).unwrap();
        let c_sname = CString::new("pipewire-relay-test").unwrap();
        let opts = LwasapiPwBrokerOpts {
            runtime_dir: c_rdir.as_ptr(),
            socket_name: c_sname.as_ptr(),
            ignore_zero_flag: true,
        };
        let mut brk: *mut PwBrokerHandle = ptr::null_mut();
        assert_eq!(unsafe { lwasapi_pw_broker_start(&mut brk, &opts) }, 0);
        unsafe { lwasapi_pw_broker_set_preview_ex(brk, true, 5000, 64, 64, 0) };

        let sock_path = format!("{}/pipewire-relay-test", tmp_dir);
        let prod = UnixStream::connect(&sock_path).unwrap();
        let cons = UnixStream::connect(&sock_path).unwrap();
        let _ = prod.set_nonblocking(true);
        let _ = cons.set_nonblocking(true);

        // Hello + CreateNode(Video/Source) on producer and Video/Sink on consumer
        let send_hello_and_node = |s: &UnixStream, class: &str, name: &str| {
            let mut b = PodBuilder::new(64);
            b.push_struct();
            b.int(4);
            b.pop();
            sendmsg_all_with_fds(s.as_raw_fd(), &frame_pw_msg(0, PW_CORE_M_HELLO, 0, 0, &b.into_vec()), &[]).unwrap();

            let mut props = Dict::default();
            props.set_str("media.type", "Video");
            props.set_str("media.class", class);
            props.set_str("node.name", name);
            let mut cn = PodBuilder::new(256);
            cn.push_struct();
            cn.str_val(Some("client-node"));
            cn.str_val(Some("PipeWire:Interface:ClientNode"));
            cn.int(6);
            cn.dict(&props);
            cn.int(3);
            cn.pop();
            sendmsg_all_with_fds(s.as_raw_fd(), &frame_pw_msg(0, PW_CORE_M_CREATE_OBJ, 1, 0, &cn.into_vec()), &[]).unwrap();

            // PortUpdate with EnumFormat 64x64 BGRx
            let mut fmt_b = PodBuilder::new(256);
            fmt_b.push_object(OBJ_FORMAT, PARAM_ENUM_FORMAT);
            fmt_b.prop(1, 0);
            fmt_b.id(2);
            fmt_b.prop(2, 0);
            fmt_b.id(1);
            fmt_b.prop(0x20001, 0);
            fmt_b.id(VF_BGRX);
            fmt_b.prop(0x20003, 0);
            fmt_b.rect(64, 64);
            fmt_b.pop();
            let fmt_pod = fmt_b.into_vec();

            let dir = if class.contains("Source") { 1 } else { 0 };
            let mut pu = PodBuilder::new(512);
            pu.push_struct();
            pu.int(dir);
            pu.int(0);
            pu.int(1);
            pu.int(1);
            pu.raw_pod(&fmt_pod);
            pu.none();
            pu.pop();
            sendmsg_all_with_fds(
                s.as_raw_fd(),
                &frame_pw_msg(3, PW_CN_M_PORT_UPDATE, 2, 0, &pu.into_vec()),
                &[],
            )
            .unwrap();
        };

        send_hello_and_node(&prod, "Video/Source", "test-cam");
        send_hello_and_node(&cons, "Stream/Input/Video", "test-sink");

        // Wait until link becomes active (state 4) via fill_debug
        let t0 = mono_ns();
        let mut active = false;
        while !active && mono_ns().saturating_sub(t0) < 1_500_000_000 {
            let mut snap: LwasapiDbgPwSnapshot = unsafe { mem::zeroed() };
            unsafe { lwasapi_pw_broker_fill_debug(brk, &mut snap) };
            if snap.active_casting == 1 {
                active = true;
                break;
            }
            thread::sleep(std::time::Duration::from_millis(15));
        }
        assert!(active, "video producer and consumer should auto-link and negotiate");

        // Pump a BGRx test pattern into producer's first allocated buffer & trigger done_efd!
        {
            let h = unsafe { &*brk };
            let mut st = h.state.lock().unwrap();
            let pidx = st
                .nodes
                .iter()
                .position(|n| n.in_use && n.direction == PWB_DIR_OUT)
                .unwrap();
            if let Some(f) = st.nodes[pidx].bufs[0].fd.as_ref() {
                let mut fdup = fs::File::from(dup_fd_cloexec(f.as_raw_fd()).unwrap());
                fdup.seek(SeekFrom::Start(0)).unwrap();
                let mut px = vec![0u8; 64 * 64 * 4];
                for c in px.as_chunks_mut::<4>().0 {
                    c[0] = 0x11;
                    c[1] = 0x55;
                    c[2] = 0xaa;
                    c[3] = 0xff;
                }
                fdup.write_all(&px).unwrap();
            }
            st.nodes[pidx].io_write_status_and_buf(PWB_STATUS_HAVE_DATA, 0);
            st.on_prod_done_event(pidx);

            let cidx = st
                .nodes
                .iter()
                .position(|n| n.in_use && n.direction == PWB_DIR_IN)
                .unwrap();
            if let Some(arfd) = st.nodes[pidx].act_readfd.as_ref() {
                eventfd_drain(arfd.as_raw_fd());
            }
            st.nodes[cidx].io_write_status_and_buf(PWB_STATUS_NEED_DATA, 0);
            st.on_cons_done_event(cidx);
            assert_eq!(
                st.nodes[pidx].io_read_status_and_buf(),
                Some((PWB_STATUS_NEED_DATA, 0))
            );
            if let Some(arfd) = st.nodes[pidx].act_readfd.as_ref() {
                let mut pfd = [PollFd {
                    fd: arfd.as_raw_fd(),
                    events: POLLIN,
                    revents: 0,
                }];
                assert_eq!(do_poll(&mut pfd, 0), 0, "on_cons_done_event must not wake prod.act_readfd");
            }

            // Verify preview SHM received converted frame
            let mut buf_px = [0u8; 4];
            if let Some(m) = st.preview_shm_map.as_ref() {
                let mut rfile =
                    fs::File::from(dup_fd_cloexec(st.preview_shm_fd.as_ref().unwrap().as_raw_fd()).unwrap());
                rfile.seek(SeekFrom::Start(520)).unwrap();
                rfile.read_exact(&mut buf_px).unwrap();
                assert_eq!(m.as_slice()[0..4], LWASAPI_PW_PREVIEW_MAGIC.to_le_bytes());
            }
            assert_eq!(&buf_px[..3], &[0x11, 0x55, 0xaa]);
        }

        let mut snap: LwasapiDbgPwSnapshot = unsafe { mem::zeroed() };
        unsafe { lwasapi_pw_broker_fill_debug(brk, &mut snap) };
        assert!(snap.frames_relayed_total >= 1);

        // Verify relay FPS meter updates actual_fps_x100 even when live preview is disabled
        unsafe { lwasapi_pw_broker_set_preview(brk, false, 0) };
        {
            let h = unsafe { &*brk };
            let mut st = h.state.lock().unwrap();
            let pidx = st
                .nodes
                .iter()
                .position(|n| n.in_use && n.direction == PWB_DIR_OUT)
                .unwrap();
            let cidx = st
                .nodes
                .iter()
                .position(|n| n.in_use && n.direction == PWB_DIR_IN)
                .unwrap();
            st.nodes[cidx].io_write_status_and_buf(PWB_STATUS_NEED_DATA, 0);
            st.relay_fps_win_ns = mono_ns().saturating_sub(500_000_000);
            st.relay_fps_win_cnt = 29;
            st.nodes[pidx].io_write_status_and_buf(PWB_STATUS_HAVE_DATA, 0);
            st.on_prod_done_event(pidx);
        }
        let mut snap_noprev: LwasapiDbgPwSnapshot = unsafe { mem::zeroed() };
        unsafe { lwasapi_pw_broker_fill_debug(brk, &mut snap_noprev) };
        assert_eq!(snap_noprev.preview_requested, 0);
        assert!(
            snap_noprev.actual_fps_x100 >= 5000,
            "expected zero-preview relay FPS >= 50.00, got {}",
            snap_noprev.actual_fps_x100
        );

        // Verify SPA_IO_AsyncBuffers dual-slot relay (`slot1` -> `slot0`) & non-clobbering recycle
        {
            let h = unsafe { &*brk };
            let mut st = h.state.lock().unwrap();
            let pidx = st
                .nodes
                .iter()
                .position(|n| n.in_use && n.direction == PWB_DIR_OUT)
                .unwrap();
            let cidx = st
                .nodes
                .iter()
                .position(|n| n.in_use && n.direction == PWB_DIR_IN)
                .unwrap();
            st.nodes[cidx].io_write_all_slots(PWB_STATUS_OK, SPA_ID_INVALID);
            st.nodes[pidx].io_write_all_slots(PWB_STATUS_NEED_DATA, SPA_ID_INVALID);
            st.nodes[pidx].io_write_slot(1, PWB_STATUS_HAVE_DATA, 1);
            st.on_prod_done_event(pidx);
            assert_eq!(
                st.nodes[cidx].io_read_status_and_buf(),
                Some((PWB_STATUS_HAVE_DATA, 1)),
                "producer slot1 frame must relay to consumer"
            );
            assert_eq!(
                st.nodes[pidx].io_read_slot(1),
                Some((PWB_STATUS_OK, 1)),
                "consumed async slot1 must clear HAVE_DATA"
            );

            // Simulate next producer frame in slot0 arriving just before consumer returns buf 1
            st.nodes[pidx].io_write_slot(0, PWB_STATUS_HAVE_DATA, 2);
            st.nodes[cidx].io_write_status_and_buf(PWB_STATUS_NEED_DATA, 1);
            st.on_cons_done_event(cidx);
            assert_eq!(
                st.nodes[pidx].io_read_slot(0),
                Some((PWB_STATUS_HAVE_DATA, 2)),
                "consumer recycle must not clobber unconsumed slot0 frame"
            );
            assert_eq!(
                st.nodes[pidx].io_read_slot(1),
                Some((PWB_STATUS_NEED_DATA, 1)),
                "returned buf 1 must recycle into originating slot1"
            );

            st.on_prod_done_event(pidx);
            assert_eq!(
                st.nodes[cidx].io_read_status_and_buf(),
                Some((PWB_STATUS_HAVE_DATA, 2)),
                "slot0 frame must relay after slot1 recycle"
            );
        }

        unsafe { lwasapi_pw_broker_stop(brk) };
        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn pw_node_async_slot_helpers_handle_single_and_coalesced_slots() {
        let fd = create_memfd("salsa-pw-slot-test", PWB_IO_BUFFERS_SIZE).unwrap();
        let map = MappedMem::map_fd(fd.as_raw_fd(), PWB_IO_BUFFERS_SIZE, true, 0);
        let mut nd = PwNode {
            io_mem_fd: Some(fd),
            io_map: map,
            ..Default::default()
        };
        nd.io_write_all_slots(PWB_STATUS_NEED_DATA, SPA_ID_INVALID);

        // Initial Frame #0 in slot1 (`(cycle + 1) & 1` with cycle=0)
        nd.io_write_slot(1, PWB_STATUS_HAVE_DATA, 0);
        assert_eq!(nd.io_take_ready_slot(), Some((1, 0)));
        assert_eq!(nd.io_read_slot(1), Some((PWB_STATUS_OK, 0)));

        // Coalesced wake with both slots populated (`slot0` first, wrapped to `slot1`)
        nd.io_write_slot(0, PWB_STATUS_HAVE_DATA, 1);
        nd.io_write_slot(1, PWB_STATUS_HAVE_DATA, 2);
        assert_eq!(nd.io_take_ready_slot(), Some((1, 2)));
        assert_eq!(nd.io_read_slot(0), Some((PWB_STATUS_NEED_DATA, 1)));
        assert_eq!(nd.io_read_slot(1), Some((PWB_STATUS_OK, 2)));
    }
}

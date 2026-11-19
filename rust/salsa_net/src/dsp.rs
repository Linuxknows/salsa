#![allow(unsafe_code)]
#![allow(clippy::missing_safety_doc)]

use core::ffi::c_void;
use core::mem;
use core::ptr;
use core::slice;
use core::sync::atomic::{AtomicU32, Ordering};
use std::f64::consts::PI;
use std::sync::Mutex;

use crate::c_abi::{
    LwasapiEqConfig, LwasapiMicApoCfg, LwasapiParamEqExt, LwasapiShmRing, SalsaMicCubicState,
    LWASAPI_EQ_BANDS, LWASAPI_FILTER_HIGH_SHELF, LWASAPI_FILTER_LOW_SHELF, LWASAPI_MAX_CHANNELS,
    LWASAPI_RING_MASK, SALSA_PACK_F32_LE, SALSA_PACK_S16_LE, SALSA_PACK_S24_3LE, SALSA_PACK_S24_LE,
    SALSA_PACK_S32_LE,
};
use crate::state_conf::K_EQ_FREQS;
use crate::unix_sys::{dl_open_any, dl_sym};

const MIC_RN_FRAME: usize = 480;
const MIC_RN_CAP: usize = 8192;
const MIC_RN_MASK: usize = MIC_RN_CAP - 1;
const INIT_NEG60_BITS: u32 = (-60.0f32).to_bits();

#[derive(Clone, Copy)]
struct BiquadFilter {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: [f64; LWASAPI_MAX_CHANNELS],
    z2: [f64; LWASAPI_MAX_CHANNELS],
    last_rate: u32,
    last_type: u32,
    last_freq_hz: f32,
    last_q: f32,
    last_gain_db: f32,
    coeffs_valid: bool,
}

impl BiquadFilter {
    const fn new() -> Self {
        Self {
            b0: 0.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: [0.0; LWASAPI_MAX_CHANNELS],
            z2: [0.0; LWASAPI_MAX_CHANNELS],
            last_rate: 0,
            last_type: 0,
            last_freq_hz: 0.0,
            last_q: 0.0,
            last_gain_db: 0.0,
            coeffs_valid: false,
        }
    }

    fn reset_state(&mut self) {
        self.z1 = [0.0; LWASAPI_MAX_CHANNELS];
        self.z2 = [0.0; LWASAPI_MAX_CHANNELS];
        self.coeffs_valid = false;
    }

    fn reset_all(&mut self) {
        *self = Self::new();
    }

    fn compute_parametric(&mut self, ftype: u32, mut freq: f64, mut rate: f64, q_in: f32, gain_db: f32) {
        if !rate.is_finite() || rate < 1.0 {
            rate = 48_000.0;
        }
        if !freq.is_finite() || freq < 10.0 {
            freq = 10.0;
        }
        if freq >= rate * 0.48 {
            freq = rate * 0.48;
        }
        let q = if !q_in.is_finite() || q_in < 0.2 {
            0.2
        } else {
            (q_in as f64).min(20.0)
        };
        let gdb = if gain_db.is_finite() {
            (gain_db as f64).clamp(-36.0, 36.0)
        } else {
            0.0
        };
        let a = 10.0f64.powf(gdb / 40.0);
        let w0 = 2.0 * PI * freq / rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        let (b0, b1, b2, a0, a1, a2) = if ftype == LWASAPI_FILTER_LOW_SHELF {
            let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
            (
                a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0),
                a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha),
                (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0),
                (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha,
            )
        } else if ftype == LWASAPI_FILTER_HIGH_SHELF {
            let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;
            (
                a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0),
                a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha),
                (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha,
                2.0 * ((a - 1.0) - (a + 1.0) * cos_w0),
                (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha,
            )
        } else {
            (
                1.0 + alpha * a,
                -2.0 * cos_w0,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos_w0,
                1.0 - alpha / a,
            )
        };

        let inv_a0 = 1.0 / a0;
        self.b0 = b0 * inv_a0;
        self.b1 = b1 * inv_a0;
        self.b2 = b2 * inv_a0;
        self.a1 = a1 * inv_a0;
        self.a2 = a2 * inv_a0;
        if !self.b0.is_finite()
            || !self.b1.is_finite()
            || !self.b2.is_finite()
            || !self.a1.is_finite()
            || !self.a2.is_finite()
        {
            self.b0 = 1.0;
            self.b1 = 0.0;
            self.b2 = 0.0;
            self.a1 = 0.0;
            self.a2 = 0.0;
        }
    }

    fn compute_hpf(&mut self, mut freq: f64, mut rate: f64) {
        if !rate.is_finite() || rate < 1.0 {
            rate = 48_000.0;
        }
        if !freq.is_finite() || freq < 20.0 {
            freq = 20.0;
        }
        if freq >= rate * 0.45 {
            freq = rate * 0.45;
        }
        let w0 = 2.0 * PI * freq / rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * core::f64::consts::FRAC_1_SQRT_2);
        let b0 = (1.0 + cos_w0) * 0.5;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        let inv_a0 = 1.0 / a0;
        self.b0 = b0 * inv_a0;
        self.b1 = b1 * inv_a0;
        self.b2 = b2 * inv_a0;
        self.a1 = a1 * inv_a0;
        self.a2 = a2 * inv_a0;
        if !self.b0.is_finite()
            || !self.b1.is_finite()
            || !self.b2.is_finite()
            || !self.a1.is_finite()
            || !self.a2.is_finite()
        {
            self.b0 = 1.0;
            self.b1 = 0.0;
            self.b2 = 0.0;
            self.a1 = 0.0;
            self.a2 = 0.0;
        }
    }

    fn compute_bandpass(&mut self, mut freq: f64, mut rate: f64, q_in: f32) {
        if !rate.is_finite() || rate < 1.0 {
            rate = 48_000.0;
        }
        if !freq.is_finite() || freq < 10.0 {
            freq = 10.0;
        }
        if freq >= rate * 0.48 {
            freq = rate * 0.48;
        }
        let q = if !q_in.is_finite() || q_in < 0.2 {
            0.2
        } else {
            (q_in as f64).min(20.0)
        };
        let w0 = 2.0 * PI * freq / rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);
        let b0 = alpha;
        let b1 = 0.0;
        let b2 = -alpha;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;
        let inv_a0 = 1.0 / a0;
        self.b0 = b0 * inv_a0;
        self.b1 = b1 * inv_a0;
        self.b2 = b2 * inv_a0;
        self.a1 = a1 * inv_a0;
        self.a2 = a2 * inv_a0;
    }
}

struct DspTelem {
    band_levels_db: [AtomicU32; LWASAPI_EQ_BANDS],
    dyn_delta_db: [AtomicU32; LWASAPI_EQ_BANDS],
    output_peak_l: AtomicU32,
    output_peak_r: AtomicU32,
    mic_apo_peak_in: AtomicU32,
    mic_apo_peak_out: AtomicU32,
    mic_rnnoise_active: AtomicU32,
}

impl DspTelem {
    const fn new() -> Self {
        Self {
            band_levels_db: [const { AtomicU32::new(INIT_NEG60_BITS) }; LWASAPI_EQ_BANDS],
            dyn_delta_db: [const { AtomicU32::new(0) }; LWASAPI_EQ_BANDS],
            output_peak_l: AtomicU32::new(0),
            output_peak_r: AtomicU32::new(0),
            mic_apo_peak_in: AtomicU32::new(0),
            mic_apo_peak_out: AtomicU32::new(0),
            mic_rnnoise_active: AtomicU32::new(0),
        }
    }
}

static GLOBAL_TELEM: DspTelem = DspTelem::new();

struct EqDspState {
    eq_filters: [BiquadFilter; LWASAPI_EQ_BANDS],
    spec_filters: [BiquadFilter; LWASAPI_EQ_BANDS],
    spec_last_freqs: [f32; LWASAPI_EQ_BANDS],
    spec_last_rate: u32,
    band_env: [f32; LWASAPI_EQ_BANDS],
    eq_band_levels_db: [f32; LWASAPI_EQ_BANDS],
    eq_dyn_delta_db: [f32; LWASAPI_EQ_BANDS],
    output_peak_l: f32,
    output_peak_r: f32,
    xfeed_lp_l: f64,
    xfeed_lp_r: f64,
    xfeed_dly_l: [f32; 64],
    xfeed_dly_r: [f32; 64],
    xfeed_dly_idx: u32,
}

impl EqDspState {
    const fn new() -> Self {
        Self {
            eq_filters: [BiquadFilter::new(); LWASAPI_EQ_BANDS],
            spec_filters: [BiquadFilter::new(); LWASAPI_EQ_BANDS],
            spec_last_freqs: [0.0; LWASAPI_EQ_BANDS],
            spec_last_rate: 0,
            band_env: [0.0; LWASAPI_EQ_BANDS],
            eq_band_levels_db: [-60.0; LWASAPI_EQ_BANDS],
            eq_dyn_delta_db: [0.0; LWASAPI_EQ_BANDS],
            output_peak_l: 0.0,
            output_peak_r: 0.0,
            xfeed_lp_l: 0.0,
            xfeed_lp_r: 0.0,
            xfeed_dly_l: [0.0; 64],
            xfeed_dly_r: [0.0; 64],
            xfeed_dly_idx: 0,
        }
    }
}

static EQ_DSP: Mutex<EqDspState> = Mutex::new(EqDspState::new());

type RnnoiseCreateFn = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
type RnnoiseDestroyFn = unsafe extern "C" fn(*mut c_void);
type RnnoiseProcessFn = unsafe extern "C" fn(*mut c_void, *mut f32, *const f32) -> f32;

struct MicRnnoiseCtx {
    _lib: *mut c_void,
    create_fn: Option<RnnoiseCreateFn>,
    destroy_fn: Option<RnnoiseDestroyFn>,
    process_fn: Option<RnnoiseProcessFn>,
    st: *mut c_void,
    probed: bool,
    available: bool,
    in_ring: [f32; MIC_RN_CAP],
    in_count: usize,
    out_ring: [f32; MIC_RN_CAP],
    dry_ring: [f32; MIC_RN_CAP],
    vad_ring: [f32; MIC_RN_CAP],
    out_rd: u32,
    out_wr: u32,
    in_last: f32,
    out_last: f32,
    dry_last: f32,
    vad_last: f32,
    in_phase: f64,
    out_phase: f64,
    last_rate: u32,
}

unsafe impl Send for MicRnnoiseCtx {}

impl MicRnnoiseCtx {
    const fn new() -> Self {
        Self {
            _lib: ptr::null_mut(),
            create_fn: None,
            destroy_fn: None,
            process_fn: None,
            st: ptr::null_mut(),
            probed: false,
            available: false,
            in_ring: [0.0; MIC_RN_CAP],
            in_count: 0,
            out_ring: [0.0; MIC_RN_CAP],
            dry_ring: [0.0; MIC_RN_CAP],
            vad_ring: [0.0; MIC_RN_CAP],
            out_rd: 0,
            out_wr: 0,
            in_last: 0.0,
            out_last: 0.0,
            dry_last: 0.0,
            vad_last: 0.0,
            in_phase: 0.0,
            out_phase: 0.0,
            last_rate: 0,
        }
    }

    fn ensure_loaded(&mut self) {
        if self.probed {
            return;
        }
        self.probed = true;
        const CANDS: &[&str] = &[
            "librnnoise.so.0",
            "/usr/lib/librnnoise.so.0",
            "/usr/local/lib/librnnoise.so.0",
        ];
        let lib = unsafe { dl_open_any(CANDS) };
        if lib.is_null() {
            return;
        }
        self._lib = lib;
        let c_sym = unsafe { dl_sym(lib, "rnnoise_create") };
        let d_sym = unsafe { dl_sym(lib, "rnnoise_destroy") };
        let p_sym = unsafe { dl_sym(lib, "rnnoise_process_frame") };
        if !c_sym.is_null() && !d_sym.is_null() && !p_sym.is_null() {
            let create_fn: RnnoiseCreateFn = unsafe { mem::transmute(c_sym) };
            let destroy_fn: RnnoiseDestroyFn = unsafe { mem::transmute(d_sym) };
            let process_fn: RnnoiseProcessFn = unsafe { mem::transmute(p_sym) };
            self.create_fn = Some(create_fn);
            self.destroy_fn = Some(destroy_fn);
            self.process_fn = Some(process_fn);
            self.st = unsafe { create_fn(ptr::null_mut()) };
            self.available = !self.st.is_null();
        }
        if self.available {
            eprintln!(
                "[LWASAPI-Salsa] Capture Mic APO: Neural RNNoise denoiser loaded (librnnoise.so.0)."
            );
        }
    }

    fn run_mono(
        &mut self,
        mono: &mut [f32],
        out_dry_aligned: &mut [f32],
        out_vad_aligned: &mut [f32],
        frames: usize,
        rate: u32,
    ) {
        if frames == 0 || !self.available || self.st.is_null() {
            return;
        }
        let Some(process_fn) = self.process_fn else {
            return;
        };
        if self.last_rate != rate {
            self.in_count = 0;
            self.out_rd = 0;
            self.out_wr = MIC_RN_FRAME as u32;
            self.out_ring[..MIC_RN_FRAME].fill(0.0);
            self.dry_ring[..MIC_RN_FRAME].fill(0.0);
            self.vad_ring[..MIC_RN_FRAME].fill(0.0);
            self.in_last = 0.0;
            self.out_last = 0.0;
            self.dry_last = 0.0;
            self.vad_last = 0.0;
            self.in_phase = 0.0;
            self.out_phase = 0.0;
            self.last_rate = rate;
        }

        if rate == 48_000 {
            let room = MIC_RN_CAP - self.in_count;
            let take = frames.min(room);
            for &s in &mono[..take] {
                self.in_ring[self.in_count] = s * 32768.0;
                self.in_count += 1;
            }
        } else {
            let step_in = (rate as f64) / 48_000.0;
            let mut ph = self.in_phase;
            while ph < (frames as f64) && self.in_count < MIC_RN_CAP {
                let i0 = ph.floor() as isize;
                let frac = ph - (i0 as f64);
                let s0 = if i0 < 0 {
                    self.in_last
                } else {
                    mono[i0 as usize]
                };
                let i1 = (i0 + 1) as usize;
                let s1 = if i1 < frames {
                    mono[i1]
                } else {
                    mono[frames - 1]
                };
                let y = s0 + (s1 - s0) * (frac as f32);
                self.in_ring[self.in_count] = y * 32768.0;
                self.in_count += 1;
                ph += step_in;
            }
            self.in_phase = ph - (frames as f64);
            self.in_last = mono[frames - 1];
        }

        let mut tmp_out = [0.0f32; MIC_RN_FRAME];
        while self.in_count >= MIC_RN_FRAME {
            let mut vad = unsafe { process_fn(self.st, tmp_out.as_mut_ptr(), self.in_ring.as_ptr()) };
            if !(0.0..=1.0).contains(&vad) {
                vad = if vad > 1.0 { 1.0 } else { 0.0 };
            }
            for k in 0..MIC_RN_FRAME {
                if (self.out_wr.wrapping_sub(self.out_rd) as usize) < MIC_RN_CAP {
                    let slot = (self.out_wr as usize) & MIC_RN_MASK;
                    self.out_ring[slot] = tmp_out[k] * (1.0 / 32768.0);
                    self.dry_ring[slot] = self.in_ring[k] * (1.0 / 32768.0);
                    self.vad_ring[slot] = vad;
                    self.out_wr = self.out_wr.wrapping_add(1);
                }
            }
            let rem = self.in_count - MIC_RN_FRAME;
            if rem > 0 {
                self.in_ring.copy_within(MIC_RN_FRAME..self.in_count, 0);
            }
            self.in_count = rem;
        }

        if rate == 48_000 {
            for f in 0..frames {
                if self.out_wr.wrapping_sub(self.out_rd) > 0 {
                    let slot = (self.out_rd as usize) & MIC_RN_MASK;
                    let y = self.out_ring[slot];
                    let d = self.dry_ring[slot];
                    let v = self.vad_ring[slot];
                    self.out_rd = self.out_rd.wrapping_add(1);
                    self.out_last = y;
                    self.dry_last = d;
                    self.vad_last = v;
                    mono[f] = y;
                    out_dry_aligned[f] = d;
                    out_vad_aligned[f] = v;
                } else {
                    mono[f] = self.out_last;
                    out_dry_aligned[f] = self.dry_last;
                    out_vad_aligned[f] = self.vad_last;
                }
            }
            if self.out_wr.wrapping_sub(self.out_rd) > (MIC_RN_FRAME as u32) * 2 {
                self.out_rd = self.out_wr.wrapping_sub(MIC_RN_FRAME as u32);
            }
        } else {
            let step_out = 48_000.0 / (rate as f64);
            let mut ph = self.out_phase;
            for f in 0..frames {
                let avail = self.out_wr.wrapping_sub(self.out_rd);
                let mut y = self.out_last;
                let mut d = self.dry_last;
                let mut v = self.vad_last;
                if avail >= 2 {
                    let s0 = (self.out_rd as usize) & MIC_RN_MASK;
                    let s1 = (self.out_rd.wrapping_add(1) as usize) & MIC_RN_MASK;
                    y = self.out_ring[s0] + (self.out_ring[s1] - self.out_ring[s0]) * (ph as f32);
                    d = self.dry_ring[s0] + (self.dry_ring[s1] - self.dry_ring[s0]) * (ph as f32);
                    v = self.vad_ring[s0];
                    self.out_last = y;
                    self.dry_last = d;
                    self.vad_last = v;
                } else if avail == 1 {
                    let s0 = (self.out_rd as usize) & MIC_RN_MASK;
                    y = self.out_ring[s0];
                    d = self.dry_ring[s0];
                    v = self.vad_ring[s0];
                    self.out_last = y;
                    self.dry_last = d;
                    self.vad_last = v;
                }
                mono[f] = y;
                out_dry_aligned[f] = d;
                out_vad_aligned[f] = v;
                ph += step_out;
                let mut adv = ph.floor() as u32;
                if adv > 0 {
                    if adv > avail {
                        adv = avail;
                    }
                    self.out_rd = self.out_rd.wrapping_add(adv);
                    ph -= adv as f64;
                }
            }
            self.out_phase = ph;
            if self.out_wr.wrapping_sub(self.out_rd) > (MIC_RN_FRAME as u32) * 2 {
                self.out_rd = self.out_wr.wrapping_sub(MIC_RN_FRAME as u32);
            }
        }
    }
}

impl Drop for MicRnnoiseCtx {
    fn drop(&mut self) {
        if let Some(destroy_fn) = self.destroy_fn {
            if !self.st.is_null() {
                unsafe { destroy_fn(self.st) };
                self.st = ptr::null_mut();
            }
        }
    }
}

struct MicApoState {
    peak_in: f32,
    peak_out: f32,
    hpf_flt: BiquadFilter,
    demud_flt: BiquadFilter,
    pres_flt: BiquadFilter,
    clarity_flt: BiquadFilter,
    gate_env: f32,
    gate_gain: f32,
    comp_env: f32,
    comp_gain: f32,
    vad_env: f32,
    vad_hold_samples: u32,
    silence_gain: f32,
    dc_x1: f64,
    dc_y1: f64,
    ch_env_l: f32,
    ch_env_r: f32,
    rn: MicRnnoiseCtx,
    mono: Vec<f32>,
    dry_aligned: Vec<f32>,
    vad_aligned: Vec<f32>,
}

impl MicApoState {
    const fn new() -> Self {
        Self {
            peak_in: 0.0,
            peak_out: 0.0,
            hpf_flt: BiquadFilter::new(),
            demud_flt: BiquadFilter::new(),
            pres_flt: BiquadFilter::new(),
            clarity_flt: BiquadFilter::new(),
            gate_env: 0.0,
            gate_gain: 1.0,
            comp_env: 0.0,
            comp_gain: 1.0,
            vad_env: 0.0,
            vad_hold_samples: 0,
            silence_gain: 1.0,
            dc_x1: 0.0,
            dc_y1: 0.0,
            ch_env_l: 0.0,
            ch_env_r: 0.0,
            rn: MicRnnoiseCtx::new(),
            mono: Vec::new(),
            dry_aligned: Vec::new(),
            vad_aligned: Vec::new(),
        }
    }
}

static MIC_APO_DSP: Mutex<MicApoState> = Mutex::new(MicApoState::new());

fn process_crossfeed(
    st: &mut EqDspState,
    buf: &mut [f32],
    frames: usize,
    ch: usize,
    rate: u32,
    eq_cfg: &LwasapiEqConfig,
    param_eq: &LwasapiParamEqExt,
) {
    if ch < 2 || eq_cfg.enabled == 0 || param_eq.crossfeed_enabled == 0 {
        st.xfeed_lp_l = 0.0;
        st.xfeed_lp_r = 0.0;
        return;
    }
    let mut lvl = param_eq.crossfeed_level;
    if lvl < 0.02 {
        return;
    }
    if !(0.02..=1.0).contains(&lvl) {
        lvl = 1.0;
    }
    let mut fc = param_eq.crossfeed_cutoff_hz;
    if !(300..=1500).contains(&fc) {
        fc = 700;
    }

    let lp_a = (-2.0 * PI * (fc as f64) / (rate as f64)).exp();
    let lp_b = 1.0 - lp_a;
    let feed = 0.26 * lvl;
    let direct = 1.0 - 0.14 * lvl;
    let dly_len = ((0.00026 * (rate as f64)) as u32).clamp(1, 63);

    for f in 0..frames {
        let base = f * ch;
        let xl = buf[base];
        let xr = buf[base + 1];
        let w_pos = (st.xfeed_dly_idx & 63) as usize;
        st.xfeed_dly_l[w_pos] = xl;
        st.xfeed_dly_r[w_pos] = xr;
        let r_pos = (st.xfeed_dly_idx.wrapping_sub(dly_len) & 63) as usize;
        let dl = st.xfeed_dly_l[r_pos];
        let dr = st.xfeed_dly_r[r_pos];
        st.xfeed_dly_idx = st.xfeed_dly_idx.wrapping_add(1);

        st.xfeed_lp_l = lp_b * (dl as f64) + lp_a * st.xfeed_lp_l;
        st.xfeed_lp_r = lp_b * (dr as f64) + lp_a * st.xfeed_lp_r;
        if st.xfeed_lp_l.abs() < 1e-18 {
            st.xfeed_lp_l = 0.0;
        }
        if st.xfeed_lp_r.abs() < 1e-18 {
            st.xfeed_lp_r = 0.0;
        }

        buf[base] = xl * direct + (st.xfeed_lp_r as f32) * feed;
        buf[base + 1] = xr * direct + (st.xfeed_lp_l as f32) * feed;
    }
}

#[allow(clippy::too_many_arguments)]
fn process_dynamic_eq_inner(
    st: &mut EqDspState,
    telem: &DspTelem,
    buf: &mut [f32],
    frames: usize,
    ch: usize,
    mut rate: u32,
    eq_cfg: &LwasapiEqConfig,
    param_eq: &LwasapiParamEqExt,
) {
    if frames == 0 || ch == 0 || buf.len() < frames * ch {
        return;
    }
    if rate == 0 {
        rate = 48_000;
    }
    let d_rate = rate as f64;

    for b in 0..LWASAPI_EQ_BANDS {
        let f_in = param_eq.band_freq_hz[b];
        let bfreq = if (20.0..=20_000.0).contains(&f_in) {
            f_in
        } else {
            K_EQ_FREQS[b]
        };
        if st.spec_last_rate != rate || (st.spec_last_freqs[b] - bfreq).abs() > 0.5 {
            st.spec_filters[b].reset_all();
            st.spec_filters[b].compute_bandpass(bfreq as f64, d_rate, 1.8);
            st.spec_last_freqs[b] = bfreq;
            if st.spec_last_rate != rate {
                st.eq_filters[b].reset_all();
            }
        }
    }
    st.spec_last_rate = rate;

    let preamp_db = if eq_cfg.preamp_db.is_finite() {
        eq_cfg.preamp_db.clamp(-24.0, 24.0)
    } else {
        0.0
    };
    let preamp_lin = if eq_cfg.enabled != 0 {
        10.0f32.powf(preamp_db / 20.0)
    } else {
        1.0
    };
    if eq_cfg.enabled != 0 && preamp_db.abs() > 0.01 {
        for s in &mut buf[..frames * ch] {
            *s *= preamp_lin;
        }
    }

    for b in 0..LWASAPI_EQ_BANDS {
        let sf = &mut st.spec_filters[b];
        let mut z1 = if sf.z1[0].is_finite() { sf.z1[0] } else { 0.0 };
        let mut z2 = if sf.z2[0].is_finite() { sf.z2[0] } else { 0.0 };
        let mut sum_sq = 0.0f64;

        for f in 0..frames {
            let base = f * ch;
            let mono = if ch >= 2 {
                0.5 * ((buf[base] as f64) + (buf[base + 1] as f64))
            } else {
                buf[base] as f64
            };
            let x = if mono.is_finite() { mono } else { 0.0 };
            let y = sf.b0 * x + z1;
            z1 = sf.b1 * x - sf.a1 * y + z2;
            z2 = sf.b2 * x - sf.a2 * y;
            sum_sq += y * y;
        }
        if !z1.is_finite() || z1.abs() < 1e-18 {
            z1 = 0.0;
        }
        if !z2.is_finite() || z2.abs() < 1e-18 {
            z2 = 0.0;
        }
        sf.z1[0] = z1;
        sf.z2[0] = z2;

        let band_rms = if sum_sq.is_finite() {
            ((sum_sq / (frames as f64)) as f32).sqrt() * core::f32::consts::SQRT_2
        } else {
            0.0
        };
        if !st.band_env[b].is_finite() {
            st.band_env[b] = 0.0;
        }
        let coeff = if band_rms > st.band_env[b] { 0.18 } else { 0.05 };
        st.band_env[b] += coeff * (band_rms - st.band_env[b]);
        if !st.band_env[b].is_finite() || st.band_env[b] < 1e-10 {
            st.band_env[b] = 0.0;
        }
        let db = (20.0 * (st.band_env[b] + 1e-5).log10()).clamp(-60.0, 6.0);
        st.eq_band_levels_db[b] = db;

        let mut target_delta = 0.0f32;
        if eq_cfg.enabled != 0 && eq_cfg.dynamic_mode != 0 {
            let thresh = if eq_cfg.dyn_threshold_db.is_finite() {
                eq_cfg.dyn_threshold_db.clamp(-60.0, 0.0)
            } else {
                -16.0
            };
            let ratio = if eq_cfg.dyn_ratio.is_finite() && eq_cfg.dyn_ratio >= 1.0 {
                eq_cfg.dyn_ratio.min(20.0)
            } else {
                2.0
            };
            if db > thresh {
                let over = db - thresh;
                if eq_cfg.dynamic_mode == 1 {
                    target_delta = (-over * (1.0 - (1.0 / ratio)) * 0.65).max(-10.0);
                } else if eq_cfg.dynamic_mode == 2 {
                    target_delta = (over * ((ratio - 1.0) * 0.12)).min(3.5);
                }
            }
        }
        if !st.eq_dyn_delta_db[b].is_finite() {
            st.eq_dyn_delta_db[b] = 0.0;
        }
        st.eq_dyn_delta_db[b] = 0.80 * st.eq_dyn_delta_db[b] + 0.20 * target_delta;
        telem.band_levels_db[b].store(st.eq_band_levels_db[b].to_bits(), Ordering::Relaxed);
        telem.dyn_delta_db[b].store(st.eq_dyn_delta_db[b].to_bits(), Ordering::Relaxed);
    }

    for b in 0..LWASAPI_EQ_BANDS {
        let ef = &mut st.eq_filters[b];
        if eq_cfg.enabled != 0 {
            let raw_gain = if eq_cfg.band_gains_db[b].is_finite() {
                eq_cfg.band_gains_db[b]
            } else {
                0.0
            };
            let eff_gain = (raw_gain + st.eq_dyn_delta_db[b]).clamp(-24.0, 24.0);
            if eff_gain.abs() > 0.05 {
                let q = if (0.2..=10.0).contains(&eq_cfg.band_q[b]) {
                    eq_cfg.band_q[b]
                } else {
                    1.414
                };
                let freq_hz = st.spec_last_freqs[b];
                let ftype = param_eq.band_type[b];
                if !ef.coeffs_valid
                    || ef.last_rate != rate
                    || ef.last_type != ftype
                    || (ef.last_freq_hz - freq_hz).abs() > 0.5
                    || (ef.last_q - q).abs() > 1e-4
                    || (ef.last_gain_db - eff_gain).abs() > 0.08
                {
                    ef.compute_parametric(ftype, freq_hz as f64, d_rate, q, eff_gain);
                    ef.last_rate = rate;
                    ef.last_type = ftype;
                    ef.last_freq_hz = freq_hz;
                    ef.last_q = q;
                    ef.last_gain_db = eff_gain;
                    ef.coeffs_valid = true;
                }
                let max_c = ch.min(LWASAPI_MAX_CHANNELS);
                for c in 0..max_c {
                    let mut ez1 = if ef.z1[c].is_finite() { ef.z1[c] } else { 0.0 };
                    let mut ez2 = if ef.z2[c].is_finite() { ef.z2[c] } else { 0.0 };
                    for f in 0..frames {
                        let idx = f * ch + c;
                        let x = if buf[idx].is_finite() { buf[idx] as f64 } else { 0.0 };
                        let y = ef.b0 * x + ez1;
                        ez1 = ef.b1 * x - ef.a1 * y + ez2;
                        ez2 = ef.b2 * x - ef.a2 * y;
                        buf[idx] = if y.is_finite() { y as f32 } else { 0.0 };
                    }
                    if !ez1.is_finite() || ez1.abs() < 1e-18 {
                        ez1 = 0.0;
                    }
                    if !ez2.is_finite() || ez2.abs() < 1e-18 {
                        ez2 = 0.0;
                    }
                    ef.z1[c] = ez1;
                    ef.z2[c] = ez2;
                }
            } else if ef.coeffs_valid {
                ef.reset_state();
            }
        } else if ef.coeffs_valid {
            ef.reset_state();
        }
    }

    process_crossfeed(st, buf, frames, ch, rate, eq_cfg, param_eq);

    let mut pk_l = 0.0f32;
    let mut pk_r = 0.0f32;
    let lim = eq_cfg.enabled == 0 || eq_cfg.limiter_enabled != 0;
    for f in 0..frames {
        for c in 0..ch {
            let idx = f * ch + c;
            let mut x = buf[idx];
            if lim {
                if x > 0.92 {
                    x = 0.92 + 0.08 * ((x - 0.92) / 0.08).tanh();
                } else if x < -0.92 {
                    x = -0.92 + 0.08 * ((x + 0.92) / 0.08).tanh();
                }
                buf[idx] = x;
            }
            let ax = x.abs();
            if c == 0 && ax > pk_l {
                pk_l = ax;
            }
            if c == 1 && ax > pk_r {
                pk_r = ax;
            }
        }
    }
    if ch == 1 {
        pk_r = pk_l;
    }
    st.output_peak_l = if pk_l > st.output_peak_l {
        pk_l
    } else {
        st.output_peak_l * 0.85 + pk_l * 0.15
    };
    st.output_peak_r = if pk_r > st.output_peak_r {
        pk_r
    } else {
        st.output_peak_r * 0.85 + pk_r * 0.15
    };
    telem
        .output_peak_l
        .store(st.output_peak_l.to_bits(), Ordering::Relaxed);
    telem
        .output_peak_r
        .store(st.output_peak_r.to_bits(), Ordering::Relaxed);
}

fn process_capture_mic_apo_inner(
    st: &mut MicApoState,
    telem: &DspTelem,
    mic_buf: &mut [f32],
    frames: usize,
    mut rate: u32,
    cfg: &LwasapiMicApoCfg,
) {
    if frames == 0 || mic_buf.len() < frames * 2 {
        return;
    }
    if rate == 0 {
        rate = 48_000;
    }

    let mut pk_in = 0.0f32;
    for f in 0..frames {
        let al = mic_buf[f * 2].abs();
        let ar = mic_buf[f * 2 + 1].abs();
        let m = al.max(ar);
        if m > pk_in {
            pk_in = m;
        }
    }
    st.peak_in = if pk_in > st.peak_in {
        pk_in
    } else {
        st.peak_in * 0.85 + pk_in * 0.15
    };
    telem
        .mic_apo_peak_in
        .store(st.peak_in.to_bits(), Ordering::Relaxed);

    if cfg.enabled == 0 {
        st.peak_out = st.peak_in;
        st.vad_env = 0.0;
        st.vad_hold_samples = 0;
        st.silence_gain = 1.0;
        telem
            .mic_apo_peak_out
            .store(st.peak_out.to_bits(), Ordering::Relaxed);
        telem.mic_rnnoise_active.store(0, Ordering::Relaxed);
        return;
    }

    st.mono.clear();
    st.mono.resize(frames, 0.0);
    st.dry_aligned.clear();
    st.dry_aligned.resize(frames, 0.0);
    st.vad_aligned.clear();
    st.vad_aligned.resize(frames, 0.0);

    let mut p_abs_l = 0.0f32;
    let mut p_abs_r = 0.0f32;
    for f in 0..frames {
        let al = mic_buf[f * 2].abs();
        let ar = mic_buf[f * 2 + 1].abs();
        if al > p_abs_l {
            p_abs_l = al;
        }
        if ar > p_abs_r {
            p_abs_r = ar;
        }
    }
    st.ch_env_l = 0.85 * st.ch_env_l + 0.15 * p_abs_l;
    st.ch_env_r = 0.85 * st.ch_env_r + 0.15 * p_abs_r;
    let use_left_only = st.ch_env_l > 1e-5 && st.ch_env_r < 0.12 * st.ch_env_l;
    let use_right_only = st.ch_env_r > 1e-5 && st.ch_env_l < 0.12 * st.ch_env_r;

    let dc_r = (1.0 - (2.0 * PI * 10.0) / (rate as f64)).clamp(0.90, 0.9995);
    for f in 0..frames {
        let l = mic_buf[f * 2];
        let r = mic_buf[f * 2 + 1];
        let raw = if use_left_only {
            l
        } else if use_right_only {
            r
        } else if (l - r).abs() < 1e-6 {
            l
        } else {
            0.5 * (l + r)
        };
        let x = if raw.is_finite() { raw as f64 } else { 0.0 };
        let mut y = x - st.dc_x1 + dc_r * st.dc_y1;
        st.dc_x1 = x;
        if !y.is_finite() || y.abs() < 1e-18 {
            st.dc_x1 = 0.0;
            y = 0.0;
        }
        st.dc_y1 = y;
        st.mono[f] = y as f32;
    }

    if cfg.hpf_enabled != 0 {
        let hpf_hz = if (40.0..=250.0).contains(&cfg.hpf_hz) {
            cfg.hpf_hz
        } else {
            80.0
        };
        if !st.hpf_flt.coeffs_valid
            || st.hpf_flt.last_rate != rate
            || (st.hpf_flt.last_freq_hz - hpf_hz).abs() > 0.5
        {
            st.hpf_flt.compute_hpf(hpf_hz as f64, rate as f64);
            st.hpf_flt.last_rate = rate;
            st.hpf_flt.last_freq_hz = hpf_hz;
            st.hpf_flt.coeffs_valid = true;
        }
        let mut z1 = if st.hpf_flt.z1[0].is_finite() { st.hpf_flt.z1[0] } else { 0.0 };
        let mut z2 = if st.hpf_flt.z2[0].is_finite() { st.hpf_flt.z2[0] } else { 0.0 };
        for s in &mut st.mono[..frames] {
            let x = if s.is_finite() { *s as f64 } else { 0.0 };
            let y = st.hpf_flt.b0 * x + z1;
            z1 = st.hpf_flt.b1 * x - st.hpf_flt.a1 * y + z2;
            z2 = st.hpf_flt.b2 * x - st.hpf_flt.a2 * y;
            *s = if y.is_finite() { y as f32 } else { 0.0 };
        }
        if !z1.is_finite() || z1.abs() < 1e-18 {
            z1 = 0.0;
        }
        if !z2.is_finite() || z2.abs() < 1e-18 {
            z2 = 0.0;
        }
        st.hpf_flt.z1[0] = z1;
        st.hpf_flt.z2[0] = z2;
    } else if st.hpf_flt.coeffs_valid {
        st.hpf_flt.reset_state();
    }

    let mut used_rn = false;
    if cfg.rnnoise_enabled != 0 {
        st.rn.ensure_loaded();
        if st.rn.available {
            st.rn.run_mono(
                &mut st.mono,
                &mut st.dry_aligned,
                &mut st.vad_aligned,
                frames,
                rate,
            );
            let f_rate = rate as f32;
            let vad_atk = (-1.0 / (0.003 * f_rate)).exp();
            let vad_rel = (-1.0 / (0.140 * f_rate)).exp();
            let sil_atk = (-1.0 / (0.004 * f_rate)).exp();
            let sil_rel = (-1.0 / (0.085 * f_rate)).exp();
            let hold_reset = (0.220 * f_rate) as u32;
            for f in 0..frames {
                let v_raw = st.vad_aligned[f];
                if v_raw >= 0.35 {
                    st.vad_hold_samples = hold_reset;
                } else if st.vad_hold_samples > 0 {
                    st.vad_hold_samples -= 1;
                }
                let v_tgt = if st.vad_hold_samples > 0 && v_raw < 0.48 {
                    0.48
                } else {
                    v_raw
                };
                let vc = if v_tgt > st.vad_env { vad_atk } else { vad_rel };
                st.vad_env = vc * st.vad_env + (1.0 - vc) * v_tgt;
                st.vad_aligned[f] = st.vad_env;

                let mut dry_mix = 0.0f32;
                if st.vad_env > 0.25 {
                    let t = ((st.vad_env - 0.25) / 0.60).min(1.0);
                    dry_mix = 0.10 * t;
                }
                let sil_tgt = if st.vad_env >= 0.20 { 1.0 } else { 0.125 };
                let sc = if sil_tgt > st.silence_gain {
                    sil_atk
                } else {
                    sil_rel
                };
                st.silence_gain = sc * st.silence_gain + (1.0 - sc) * sil_tgt;
                st.mono[f] = ((1.0 - dry_mix) * st.mono[f] + dry_mix * st.dry_aligned[f])
                    * st.silence_gain;
            }
            used_rn = true;
        }
    }
    telem
        .mic_rnnoise_active
        .store(u32::from(used_rn), Ordering::Relaxed);

    if cfg.gate_enabled != 0 {
        let thresh_lin = 10.0f32.powf(cfg.gate_thresh_db / 20.0);
        let close_lin = thresh_lin * 0.63;
        let f_rate = rate as f32;
        let env_atk = (-1.0 / (0.002 * f_rate)).exp();
        let env_rel = (-1.0 / (0.120 * f_rate)).exp();
        let g_atk = (-1.0 / (0.0025 * f_rate)).exp();
        let g_rel = (-1.0 / (0.140 * f_rate)).exp();
        for f in 0..frames {
            let ax = st.mono[f].abs();
            let ec = if ax > st.gate_env { env_atk } else { env_rel };
            st.gate_env = ec * st.gate_env + (1.0 - ec) * ax;
            let tgt = if used_rn {
                let v = st.vad_aligned[f];
                let open_vad = (v >= 0.28) || (st.gate_env >= thresh_lin && v >= 0.12);
                let close_vad = (v < 0.16 && st.gate_env <= close_lin) || (v < 0.09);
                if open_vad {
                    1.0
                } else if close_vad {
                    0.012
                } else {
                    st.gate_gain
                }
            } else if st.gate_env >= thresh_lin {
                1.0
            } else if st.gate_env <= close_lin {
                0.015
            } else {
                st.gate_gain
            };
            let gc = if tgt > st.gate_gain { g_atk } else { g_rel };
            st.gate_gain = gc * st.gate_gain + (1.0 - gc) * tgt;
            st.mono[f] *= st.gate_gain;
        }
    }

    if cfg.comp_enabled != 0 {
        if !st.demud_flt.coeffs_valid || st.demud_flt.last_rate != rate {
            st.demud_flt
                .compute_parametric(crate::c_abi::LWASAPI_FILTER_PEAK, 250.0, rate as f64, 1.10, -1.4);
            st.demud_flt.last_rate = rate;
            st.demud_flt.coeffs_valid = true;
        }
        let mut dz1 = if st.demud_flt.z1[0].is_finite() { st.demud_flt.z1[0] } else { 0.0 };
        let mut dz2 = if st.demud_flt.z2[0].is_finite() { st.demud_flt.z2[0] } else { 0.0 };
        for s in &mut st.mono[..frames] {
            let x = if s.is_finite() { *s as f64 } else { 0.0 };
            let y = st.demud_flt.b0 * x + dz1;
            dz1 = st.demud_flt.b1 * x - st.demud_flt.a1 * y + dz2;
            dz2 = st.demud_flt.b2 * x - st.demud_flt.a2 * y;
            *s = if y.is_finite() { y as f32 } else { 0.0 };
        }
        if !dz1.is_finite() || dz1.abs() < 1e-18 {
            dz1 = 0.0;
        }
        if !dz2.is_finite() || dz2.abs() < 1e-18 {
            dz2 = 0.0;
        }
        st.demud_flt.z1[0] = dz1;
        st.demud_flt.z2[0] = dz2;

        let clarity_db = if cfg.clarity_db.is_finite() {
            cfg.clarity_db.clamp(-6.0, 12.0)
        } else {
            0.0
        };
        if clarity_db.abs() > 0.1 {
            if !st.pres_flt.coeffs_valid
                || st.pres_flt.last_rate != rate
                || (st.pres_flt.last_gain_db - clarity_db).abs() > 0.05
            {
                st.pres_flt.compute_parametric(
                    crate::c_abi::LWASAPI_FILTER_PEAK,
                    3400.0,
                    rate as f64,
                    0.80,
                    clarity_db * 0.40,
                );
                st.pres_flt.last_rate = rate;
                st.pres_flt.last_gain_db = clarity_db;
                st.pres_flt.coeffs_valid = true;
            }
            if !st.clarity_flt.coeffs_valid
                || st.clarity_flt.last_rate != rate
                || (st.clarity_flt.last_gain_db - clarity_db).abs() > 0.05
            {
                st.clarity_flt.compute_parametric(
                    LWASAPI_FILTER_HIGH_SHELF,
                    6800.0,
                    rate as f64,
                    0.707,
                    clarity_db * 0.60,
                );
                st.clarity_flt.last_rate = rate;
                st.clarity_flt.last_gain_db = clarity_db;
                st.clarity_flt.coeffs_valid = true;
            }
            let mut pz1 = if st.pres_flt.z1[0].is_finite() { st.pres_flt.z1[0] } else { 0.0 };
            let mut pz2 = if st.pres_flt.z2[0].is_finite() { st.pres_flt.z2[0] } else { 0.0 };
            let mut sz1 = if st.clarity_flt.z1[0].is_finite() { st.clarity_flt.z1[0] } else { 0.0 };
            let mut sz2 = if st.clarity_flt.z2[0].is_finite() { st.clarity_flt.z2[0] } else { 0.0 };
            for s in &mut st.mono[..frames] {
                let x = if s.is_finite() { *s as f64 } else { 0.0 };
                let p_y = st.pres_flt.b0 * x + pz1;
                pz1 = st.pres_flt.b1 * x - st.pres_flt.a1 * p_y + pz2;
                pz2 = st.pres_flt.b2 * x - st.pres_flt.a2 * p_y;
                let s_y = st.clarity_flt.b0 * p_y + sz1;
                sz1 = st.clarity_flt.b1 * p_y - st.clarity_flt.a1 * s_y + sz2;
                sz2 = st.clarity_flt.b2 * p_y - st.clarity_flt.a2 * s_y;
                *s = if s_y.is_finite() { s_y as f32 } else { 0.0 };
            }
            if !pz1.is_finite() || pz1.abs() < 1e-18 {
                pz1 = 0.0;
            }
            if !pz2.is_finite() || pz2.abs() < 1e-18 {
                pz2 = 0.0;
            }
            if !sz1.is_finite() || sz1.abs() < 1e-18 {
                sz1 = 0.0;
            }
            if !sz2.is_finite() || sz2.abs() < 1e-18 {
                sz2 = 0.0;
            }
            st.pres_flt.z1[0] = pz1;
            st.pres_flt.z2[0] = pz2;
            st.clarity_flt.z1[0] = sz1;
            st.clarity_flt.z2[0] = sz2;
        }

        let cthresh = if cfg.comp_thresh_db.is_finite() {
            cfg.comp_thresh_db.clamp(-45.0, -1.0)
        } else {
            -15.0
        };
        let ratio = if cfg.comp_ratio.is_finite() && (1.0..=12.0).contains(&cfg.comp_ratio) {
            cfg.comp_ratio
        } else {
            2.2
        };
        let f_rate = rate as f32;
        let atk = (-1.0 / (0.004 * f_rate)).exp();
        let rel = (-1.0 / (0.090 * f_rate)).exp();
        for f in 0..frames {
            let mut x = st.mono[f];
            let ax = x.abs();
            let ec = if ax > st.comp_env { atk } else { rel };
            st.comp_env = ec * st.comp_env + (1.0 - ec) * ax;
            let env_db = 20.0 * (st.comp_env + 1e-6).log10();
            let speech_ok = if used_rn {
                if st.vad_aligned[f] > 0.30 { 1.0 } else { 0.0 }
            } else if env_db > -34.0 {
                1.0
            } else {
                0.0
            };
            let mut gain_db = 1.8 * speech_ok;
            if env_db > cthresh {
                gain_db -= (env_db - cthresh) * (1.0 - 1.0 / ratio);
            }
            let tgt = 10.0f32.powf(gain_db / 20.0);
            st.comp_gain = 0.90 * st.comp_gain + 0.10 * tgt;
            x *= st.comp_gain;
            if x > 0.94 {
                x = 0.94 + 0.05 * ((x - 0.94) / 0.05).tanh();
            } else if x < -0.94 {
                x = -0.94 + 0.05 * ((x + 0.94) / 0.05).tanh();
            }
            st.mono[f] = x;
        }
    }

    let mut pk_out = 0.0f32;
    for f in 0..frames {
        let y = st.mono[f];
        let ay = y.abs();
        if ay > pk_out {
            pk_out = ay;
        }
        mic_buf[f * 2] = y;
        mic_buf[f * 2 + 1] = y;
    }
    st.peak_out = if pk_out > st.peak_out {
        pk_out
    } else {
        st.peak_out * 0.85 + pk_out * 0.15
    };
    telem
        .mic_apo_peak_out
        .store(st.peak_out.to_bits(), Ordering::Relaxed);
}

#[inline]
pub fn soft_clip(x: f32) -> f32 {
    if x.is_nan() {
        0.0
    } else {
        x.clamp(-1.0, 1.0)
    }
}

#[inline]
pub fn shape_hw_analog_sample(x: f32, gain: f32, softknee: bool) -> f32 {
    let mut v = x * gain;
    if v.is_nan() {
        return 0.0;
    }
    if softknee {
        let av = v.abs();
        if av > 0.72 {
            let excess = av - 0.72;
            let shaped = 0.72 + 0.275 * (excess / 0.45).tanh();
            v = if v >= 0.0 { shaped } else { -shaped };
        }
        return v;
    }
    soft_clip(v)
}

#[inline]
pub fn catmull_rom(p0: f32, p1: f32, p2: f32, p3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

pub fn resample_mic_cubic_slice(
    in_buf: &[f32],
    in_frames: usize,
    out_buf: &mut [f32],
    out_frames: usize,
    st: &mut SalsaMicCubicState,
) {
    if in_frames == 0
        || out_frames == 0
        || in_buf.len() < in_frames * 2
        || out_buf.len() < out_frames * 2
    {
        return;
    }

    if in_frames == out_frames {
        out_buf[..out_frames * 2].copy_from_slice(&in_buf[..out_frames * 2]);
        for k in 0..3 {
            let src_i = (in_frames as isize) - 3 + (k as isize);
            let si = if src_i >= 0 { src_i as usize } else { 0 };
            st.hist_l[k] = in_buf[si * 2];
            st.hist_r[k] = in_buf[si * 2 + 1];
        }
        st.valid = true;
        return;
    }

    if !st.valid {
        for k in 0..3 {
            st.hist_l[k] = in_buf[0];
            st.hist_r[k] = in_buf[1];
        }
        st.valid = true;
    }

    let step = (in_frames as f64) / (out_frames as f64);
    let total = (in_frames as isize) + 3;
    for f in 0..out_frames {
        let p = 2.0 + (f as f64) * step;
        let i0 = p.floor() as isize;
        let t = (p - (i0 as f64)) as f32;
        let idx = [i0 - 1, i0, i0 + 1, i0 + 2];
        let mut sl = [0.0f32; 4];
        let mut sr = [0.0f32; 4];
        for m in 0..4 {
            let j = idx[m].clamp(0, total - 1);
            if j < 3 {
                sl[m] = st.hist_l[j as usize];
                sr[m] = st.hist_r[j as usize];
            } else {
                let u = (j - 3) as usize;
                sl[m] = in_buf[u * 2];
                sr[m] = in_buf[u * 2 + 1];
            }
        }
        out_buf[f * 2] = soft_clip(catmull_rom(sl[0], sl[1], sl[2], sl[3], t));
        out_buf[f * 2 + 1] = soft_clip(catmull_rom(sr[0], sr[1], sr[2], sr[3], t));
    }

    for k in 0..3 {
        let j = total - 3 + (k as isize);
        if j < 3 {
            let ji = if j >= 0 { j as usize } else { 0 };
            st.hist_l[k] = st.hist_l[ji];
            st.hist_r[k] = st.hist_r[ji];
        } else {
            let u = (j - 3) as usize;
            st.hist_l[k] = in_buf[u * 2];
            st.hist_r[k] = in_buf[u * 2 + 1];
        }
    }
}

pub fn scale_cap_clip_slice(buf: &mut [f32], gain: f32) {
    if (gain - 1.0).abs() > 0.001 {
        for x in buf {
            *x = soft_clip(*x * gain);
        }
    }
}

pub fn pack_pcm_bytes(
    mix_buf: &[f32],
    master_gain: f32,
    softknee: bool,
    pack_fmt: u32,
    out_bytes: &mut [u8],
) {
    let n = mix_buf.len();
    match pack_fmt {
        SALSA_PACK_S32_LE => {
            if out_bytes.len() < n * 4 {
                return;
            }
            for (k, &s) in mix_buf.iter().enumerate() {
                let v = (shape_hw_analog_sample(s, master_gain, softknee) * 2147483520.0) as i32;
                out_bytes[k * 4..k * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        SALSA_PACK_S24_LE => {
            if out_bytes.len() < n * 4 {
                return;
            }
            for (k, &s) in mix_buf.iter().enumerate() {
                let v = (shape_hw_analog_sample(s, master_gain, softknee) * 8388607.0) as i32;
                out_bytes[k * 4..k * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        SALSA_PACK_S24_3LE => {
            if out_bytes.len() < n * 3 {
                return;
            }
            for (k, &s) in mix_buf.iter().enumerate() {
                let v = (shape_hw_analog_sample(s, master_gain, softknee) * 8388607.0) as i32;
                out_bytes[k * 3] = (v & 0xFF) as u8;
                out_bytes[k * 3 + 1] = ((v >> 8) & 0xFF) as u8;
                out_bytes[k * 3 + 2] = ((v >> 16) & 0xFF) as u8;
            }
        }
        SALSA_PACK_S16_LE => {
            if out_bytes.len() < n * 2 {
                return;
            }
            for (k, &s) in mix_buf.iter().enumerate() {
                let v = (shape_hw_analog_sample(s, master_gain, softknee) * 32767.0) as i16;
                out_bytes[k * 2..k * 2 + 2].copy_from_slice(&v.to_le_bytes());
            }
        }
        _ => {
            let _ = SALSA_PACK_F32_LE;
            if out_bytes.len() < n * 4 {
                return;
            }
            for (k, &s) in mix_buf.iter().enumerate() {
                let v = shape_hw_analog_sample(s, master_gain, softknee);
                out_bytes[k * 4..k * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn salsa_dsp_process_dynamic_eq(
    buf: *mut f32,
    frames: u32,
    ch: u32,
    rate: u32,
    eq_cfg: *const LwasapiEqConfig,
    param_eq: *const LwasapiParamEqExt,
) {
    if buf.is_null() || frames == 0 || ch == 0 || eq_cfg.is_null() || param_eq.is_null() {
        return;
    }
    let Some(len) = (frames as usize).checked_mul(ch as usize) else {
        return;
    };
    let slice_buf = unsafe { slice::from_raw_parts_mut(buf, len) };
    let eq = unsafe { &*eq_cfg };
    let peq = unsafe { &*param_eq };
    if let Ok(mut st) = EQ_DSP.lock() {
        process_dynamic_eq_inner(&mut st, &GLOBAL_TELEM, slice_buf, frames as usize, ch as usize, rate, eq, peq);
    }
}

#[no_mangle]
pub unsafe extern "C" fn salsa_dsp_process_capture_mic_apo(
    mic_buf: *mut f32,
    frames: u32,
    rate: u32,
    cfg: *const LwasapiMicApoCfg,
) {
    if mic_buf.is_null() || frames == 0 || cfg.is_null() {
        return;
    }
    let Some(len) = (frames as usize).checked_mul(2) else {
        return;
    };
    let slice_buf = unsafe { slice::from_raw_parts_mut(mic_buf, len) };
    let cfg_ref = unsafe { &*cfg };
    if let Ok(mut st) = MIC_APO_DSP.lock() {
        process_capture_mic_apo_inner(
            &mut st,
            &GLOBAL_TELEM,
            slice_buf,
            frames as usize,
            rate,
            cfg_ref,
        );
    }
}

#[no_mangle]
pub unsafe extern "C" fn salsa_dsp_get_telem(
    out_band_levels_db: *mut f32,
    out_dyn_delta_db: *mut f32,
    out_pk_l: *mut f32,
    out_pk_r: *mut f32,
    out_mic_pk_in: *mut f32,
    out_mic_pk_out: *mut f32,
    out_rnnoise_active: *mut u32,
) {
    if !out_band_levels_db.is_null() {
        let out = unsafe { slice::from_raw_parts_mut(out_band_levels_db, LWASAPI_EQ_BANDS) };
        for (dst, src) in out.iter_mut().zip(GLOBAL_TELEM.band_levels_db.iter()) {
            *dst = f32::from_bits(src.load(Ordering::Relaxed));
        }
    }
    if !out_dyn_delta_db.is_null() {
        let out = unsafe { slice::from_raw_parts_mut(out_dyn_delta_db, LWASAPI_EQ_BANDS) };
        for (dst, src) in out.iter_mut().zip(GLOBAL_TELEM.dyn_delta_db.iter()) {
            *dst = f32::from_bits(src.load(Ordering::Relaxed));
        }
    }
    if !out_pk_l.is_null() {
        unsafe { *out_pk_l = f32::from_bits(GLOBAL_TELEM.output_peak_l.load(Ordering::Relaxed)) };
    }
    if !out_pk_r.is_null() {
        unsafe { *out_pk_r = f32::from_bits(GLOBAL_TELEM.output_peak_r.load(Ordering::Relaxed)) };
    }
    if !out_mic_pk_in.is_null() {
        unsafe {
            *out_mic_pk_in = f32::from_bits(GLOBAL_TELEM.mic_apo_peak_in.load(Ordering::Relaxed))
        };
    }
    if !out_mic_pk_out.is_null() {
        unsafe {
            *out_mic_pk_out = f32::from_bits(GLOBAL_TELEM.mic_apo_peak_out.load(Ordering::Relaxed))
        };
    }
    if !out_rnnoise_active.is_null() {
        unsafe { *out_rnnoise_active = GLOBAL_TELEM.mic_rnnoise_active.load(Ordering::Relaxed) };
    }
}

#[no_mangle]
pub unsafe extern "C" fn salsa_dsp_consume_session_frames(
    r: *mut LwasapiShmRing,
    io_phase: *mut f64,
    s_rate: u32,
    s_ch: u32,
    io_stream_live: *mut bool,
    io_underrun_pending: *mut bool,
    io_last_audio_ns: *mut u64,
    mix_buf: *mut f32,
    out_frames: u32,
    out_channels: u32,
    hw_rate: u32,
    replace_output: bool,
    now_ns: u64,
) {
    if io_phase.is_null()
        || io_stream_live.is_null()
        || io_underrun_pending.is_null()
        || io_last_audio_ns.is_null()
        || mix_buf.is_null()
        || out_frames == 0
        || out_channels == 0
    {
        return;
    }

    let reset_idle = || unsafe {
        *io_stream_live = false;
        *io_underrun_pending = false;
        *io_phase = 0.0;
    };

    if r.is_null() {
        reset_idle();
        return;
    }

    let active_ptr = unsafe { ptr::addr_of!((*r).active) };
    if unsafe { (*active_ptr).load(Ordering::Acquire) } == 0 {
        reset_idle();
        return;
    }

    let ch = s_ch as usize;
    if ch == 0 || ch > LWASAPI_MAX_CHANNELS {
        return;
    }
    let out_ch = out_channels as usize;
    let out_fr = out_frames as usize;
    let Some(mix_len) = out_fr.checked_mul(out_ch) else {
        return;
    };
    let mix = unsafe { slice::from_raw_parts_mut(mix_buf, mix_len) };

    let client_rate = if s_rate != 0 { s_rate } else { hw_rate };
    if client_rate == 0 || hw_rate == 0 {
        return;
    }

    let wf_ptr = unsafe { ptr::addr_of!((*r).write_frame) };
    let rf_ptr = unsafe { ptr::addr_of!((*r).read_frame) };
    let w = unsafe { (*wf_ptr).load(Ordering::Acquire) };
    let rd = unsafe { (*rf_ptr).load(Ordering::Relaxed) };
    if w < rd {
        reset_idle();
        return;
    }

    let avail = w - rd;
    let min_avail = if client_rate == hw_rate { 1 } else { 2 };
    if avail < min_avail {
        if unsafe { *io_stream_live } {
            let last_ns = unsafe { *io_last_audio_ns };
            if now_ns > last_ns && (now_ns - last_ns) > 500_000_000 {
                unsafe {
                    *io_stream_live = false;
                    *io_underrun_pending = false;
                }
            } else {
                unsafe { *io_underrun_pending = true };
            }
        }
        return;
    }

    if unsafe { *io_underrun_pending } {
        let xruns_ptr = unsafe { ptr::addr_of!((*r).xruns) };
        unsafe { (*xruns_ptr).fetch_add(1, Ordering::Relaxed) };
        unsafe { *io_underrun_pending = false };
    }

    let muted_ptr = unsafe { ptr::addr_of!((*r).muted) };
    let vol_ptr = unsafe { ptr::addr_of!((*r).volume_milli) };
    let mut vol = if unsafe { (*muted_ptr).load(Ordering::Relaxed) } != 0 {
        0.0f32
    } else {
        (unsafe { (*vol_ptr).load(Ordering::Relaxed) } as f32) / 1000.0
    };
    if !(0.0..=4.0).contains(&vol) {
        vol = if vol > 4.0 { 4.0 } else { 0.0 };
    }

    let pcm_base = unsafe { ptr::addr_of!((*r).pcm_data.0) as *const f32 };
    if client_rate == hw_rate {
        let to_read = (avail.min(out_fr as u64)) as usize;
        for f in 0..to_read {
            let slot = ((rd.wrapping_add(f as u64) as usize) & LWASAPI_RING_MASK) * ch;
            for c in 0..out_ch {
                let sample = unsafe { pcm_base.add(slot + (c % ch)).read() } * vol;
                let dst = &mut mix[f * out_ch + c];
                if replace_output {
                    *dst = sample;
                } else {
                    *dst += sample;
                }
            }
        }
        unsafe { (*rf_ptr).store(rd.wrapping_add(to_read as u64), Ordering::Release) };
    } else {
        let step = (client_rate as f64) / (hw_rate as f64);
        let mut phase = unsafe { *io_phase };
        for f in 0..out_fr {
            let i0 = phase as u64;
            if i0 + 1 >= avail {
                break;
            }
            let frac = (phase - (i0 as f64)) as f32;
            let slot0 = ((rd.wrapping_add(i0) as usize) & LWASAPI_RING_MASK) * ch;
            let slot1 = ((rd.wrapping_add(i0 + 1) as usize) & LWASAPI_RING_MASK) * ch;
            for c in 0..out_ch {
                let ch_idx = c % ch;
                let s0 = unsafe { pcm_base.add(slot0 + ch_idx).read() };
                let s1 = unsafe { pcm_base.add(slot1 + ch_idx).read() };
                let sample = (s0 + (s1 - s0) * frac) * vol;
                let dst = &mut mix[f * out_ch + c];
                if replace_output {
                    *dst = sample;
                } else {
                    *dst += sample;
                }
            }
            phase += step;
        }
        let advance = (phase as u64).min(avail);
        unsafe { *io_phase = phase - (advance as f64) };
        unsafe { (*rf_ptr).store(rd.wrapping_add(advance), Ordering::Release) };
    }

    unsafe {
        *io_stream_live = true;
        *io_last_audio_ns = now_ns;
    }
}

#[no_mangle]
pub unsafe extern "C" fn salsa_dsp_resample_mic_cubic(
    in_buf: *const f32,
    in_frames: u32,
    out_buf: *mut f32,
    out_frames: u32,
    st: *mut SalsaMicCubicState,
) {
    if in_buf.is_null() || out_buf.is_null() || in_frames == 0 || out_frames == 0 || st.is_null() {
        return;
    }
    let Some(in_len) = (in_frames as usize).checked_mul(2) else {
        return;
    };
    let Some(out_len) = (out_frames as usize).checked_mul(2) else {
        return;
    };
    let in_slice = unsafe { slice::from_raw_parts(in_buf, in_len) };
    let out_slice = unsafe { slice::from_raw_parts_mut(out_buf, out_len) };
    resample_mic_cubic_slice(
        in_slice,
        in_frames as usize,
        out_slice,
        out_frames as usize,
        unsafe { &mut *st },
    );
}

#[no_mangle]
pub unsafe extern "C" fn salsa_dsp_scale_cap_clip(buf: *mut f32, samples: u32, gain: f32) {
    if buf.is_null() || samples == 0 {
        return;
    }
    let slice_buf = unsafe { slice::from_raw_parts_mut(buf, samples as usize) };
    scale_cap_clip_slice(slice_buf, gain);
}

#[no_mangle]
pub unsafe extern "C" fn salsa_dsp_pack_pcm(
    mix_buf: *const f32,
    total_samples: u32,
    master_gain: f32,
    softknee: bool,
    pack_fmt: u32,
    out_bytes: *mut u8,
) {
    if mix_buf.is_null() || out_bytes.is_null() || total_samples == 0 {
        return;
    }
    let n = total_samples as usize;
    let out_len = match pack_fmt {
        SALSA_PACK_S32_LE | SALSA_PACK_S24_LE => n * 4,
        SALSA_PACK_S24_3LE => n * 3,
        SALSA_PACK_S16_LE => n * 2,
        _ => n * 4,
    };
    let in_slice = unsafe { slice::from_raw_parts(mix_buf, n) };
    let out_slice = unsafe { slice::from_raw_parts_mut(out_bytes, out_len) };
    pack_pcm_bytes(in_slice, master_gain, softknee, pack_fmt, out_slice);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::c_abi::LWASAPI_RING_FRAMES;
    use crate::state_conf::default_runtime_state;

    #[test]
    fn pack_pcm_formats_match_expected_little_endian() {
        let src = [0.5f32, -0.5f32, 2.0f32, -2.0f32];

        let mut s16 = [0u8; 8];
        pack_pcm_bytes(&src, 1.0, false, SALSA_PACK_S16_LE, &mut s16);
        assert_eq!(i16::from_le_bytes([s16[0], s16[1]]), 16383);
        assert_eq!(i16::from_le_bytes([s16[2], s16[3]]), -16383);
        assert_eq!(i16::from_le_bytes([s16[4], s16[5]]), 32767);
        assert_eq!(i16::from_le_bytes([s16[6], s16[7]]), -32767);

        let mut s24_3 = [0u8; 12];
        pack_pcm_bytes(&src, 1.0, false, SALSA_PACK_S24_3LE, &mut s24_3);
        let max24 = (s24_3[6] as i32) | ((s24_3[7] as i32) << 8) | ((s24_3[8] as i32) << 16);
        assert_eq!(max24, 8_388_607);

        let mut s32 = [0u8; 16];
        pack_pcm_bytes(&src, 1.0, false, SALSA_PACK_S32_LE, &mut s32);
        let full = i32::from_le_bytes([s32[8], s32[9], s32[10], s32[11]]);
        assert_eq!(full, 2_147_483_520);

        let mut f32b = [0u8; 16];
        pack_pcm_bytes(&src, 1.0, true, SALSA_PACK_F32_LE, &mut f32b);
        let shaped = f32::from_le_bytes([f32b[8], f32b[9], f32b[10], f32b[11]]);
        assert!(shaped > 0.72 && shaped < 0.996);
    }

    #[test]
    fn dynamic_eq_and_crossfeed_run_and_emit_telem() {
        let mut st = EqDspState::new();
        let telem = DspTelem::new();
        let mut rt = default_runtime_state();
        rt.eq_cfg.band_gains_db[5] = 6.0;
        rt.param_eq.crossfeed_enabled = 1;

        let frames = 256;
        let mut buf = vec![0.0f32; frames * 2];
        for blk in 0..10 {
            for f in 0..frames {
                let sample_idx = blk * frames + f;
                let phase = 2.0 * core::f32::consts::PI * 1000.0 * (sample_idx as f32) / 48_000.0;
                buf[f * 2] = 0.4 * phase.sin();
                buf[f * 2 + 1] = 0.0;
            }
            process_dynamic_eq_inner(&mut st, &telem, &mut buf, frames, 2, 48_000, &rt.eq_cfg, &rt.param_eq);
        }

        let lvl_1khz = f32::from_bits(telem.band_levels_db[5].load(Ordering::Relaxed));
        let lvl_31hz = f32::from_bits(telem.band_levels_db[0].load(Ordering::Relaxed));
        let pk_l = f32::from_bits(telem.output_peak_l.load(Ordering::Relaxed));
        let pk_r = f32::from_bits(telem.output_peak_r.load(Ordering::Relaxed));
        assert!(lvl_1khz > -20.0, "1kHz analyzer band should detect signal, got {lvl_1khz}");
        assert!(lvl_31hz < lvl_1khz - 15.0, "off-band energy should stay attenuated");
        assert!(pk_l > 0.2);
        assert!(pk_r > 0.01, "crossfeed should bleed attenuated low-passed left signal into right channel");
    }

    #[test]
    fn catmull_rom_dc_and_ramp_continuous() {
        let in_dc = vec![0.35f32; 96 * 2];
        let mut out = vec![0.0f32; 192 * 2];
        let mut st = SalsaMicCubicState {
            hist_l: [0.0; 3],
            hist_r: [0.0; 3],
            valid: false,
        };
        resample_mic_cubic_slice(&in_dc, 96, &mut out, 192, &mut st);
        assert!(st.valid);
        for &s in &out {
            assert!((s - 0.35).abs() < 1e-4);
        }
    }

    #[test]
    fn capture_mic_apo_gate_and_contour() {
        let mut st = MicApoState::new();
        let telem = DspTelem::new();
        let mut cfg = default_runtime_state().mic_apo;
        cfg.enabled = 1;
        cfg.rnnoise_enabled = 0; // Deterministic test without depending on host librnnoise.so.0
        cfg.gate_enabled = 1;
        cfg.gate_thresh_db = -25.0;

        let frames = 480;
        // Sub-threshold noise should get attenuated by hysteresis gate
        let mut quiet = vec![0.002f32; frames * 2];
        for _ in 0..20 {
            process_capture_mic_apo_inner(&mut st, &telem, &mut quiet, frames, 48_000, &cfg);
        }
        assert!(f32::from_bits(telem.mic_apo_peak_out.load(Ordering::Relaxed)) < 0.002);

        // Loud voice burst opens gate cleanly
        let mut loud = vec![0.0f32; frames * 2];
        for f in 0..frames {
            let v = 0.25 * (2.0 * core::f32::consts::PI * 300.0 * (f as f32) / 48_000.0).sin();
            loud[f * 2] = v;
            loud[f * 2 + 1] = v;
        }
        process_capture_mic_apo_inner(&mut st, &telem, &mut loud, frames, 48_000, &cfg);
        assert!(f32::from_bits(telem.mic_apo_peak_out.load(Ordering::Relaxed)) > 0.05);
    }

    #[test]
    fn consume_session_frames_native_and_resample() {
        let mut ring: Box<LwasapiShmRing> = unsafe {
            let mut u = Box::<LwasapiShmRing>::new_uninit();
            ptr::write_bytes(u.as_mut_ptr(), 0, 1);
            u.assume_init()
        };
        ring.active.store(1, Ordering::Release);
        ring.volume_milli.store(500, Ordering::Release); // 0.5x gain
        for i in 0..256 {
            let slot = (i & LWASAPI_RING_MASK) * 2;
            ring.pcm_data.0[slot] = 0.8;
            ring.pcm_data.0[slot + 1] = -0.4;
        }
        ring.write_frame.store(256, Ordering::Release);

        let mut phase = 0.0;
        let mut live = false;
        let mut underrun = false;
        let mut last_ns = 0u64;
        let mut mix = vec![0.0f32; 128 * 2];

        unsafe {
            salsa_dsp_consume_session_frames(
                &mut *ring,
                &mut phase,
                48_000,
                2,
                &mut live,
                &mut underrun,
                &mut last_ns,
                mix.as_mut_ptr(),
                128,
                2,
                48_000,
                true,
                1_000_000_000,
            );
        }
        assert!(live);
        assert_eq!(ring.read_frame.load(Ordering::Acquire), 128);
        assert!((mix[0] - 0.4).abs() < 1e-5);
        assert!((mix[1] - (-0.2)).abs() < 1e-5);

        // Resampled path 44100 -> 48000 adds onto existing mix
        unsafe {
            salsa_dsp_consume_session_frames(
                &mut *ring,
                &mut phase,
                44_100,
                2,
                &mut live,
                &mut underrun,
                &mut last_ns,
                mix.as_mut_ptr(),
                64,
                2,
                48_000,
                false,
                1_010_000_000,
            );
        }
        assert!(ring.read_frame.load(Ordering::Acquire) > 128);
        assert!(mix[0] > 0.6);

        // Next call with insufficient frames arms underrun and increments xruns on recovery
        ring.read_frame.store(256, Ordering::Release);
        unsafe {
            salsa_dsp_consume_session_frames(
                &mut *ring,
                &mut phase,
                44_100,
                2,
                &mut live,
                &mut underrun,
                &mut last_ns,
                mix.as_mut_ptr(),
                64,
                2,
                48_000,
                true,
                1_020_000_000,
            );
        }
        assert!(underrun);
        for i in 256..300 {
            let slot = (i & LWASAPI_RING_MASK) * 2;
            ring.pcm_data.0[slot] = 0.2;
            ring.pcm_data.0[slot + 1] = 0.2;
        }
        ring.write_frame.store(300, Ordering::Release);
        unsafe {
            salsa_dsp_consume_session_frames(
                &mut *ring,
                &mut phase,
                44_100,
                2,
                &mut live,
                &mut underrun,
                &mut last_ns,
                mix.as_mut_ptr(),
                16,
                2,
                48_000,
                true,
                1_030_000_000,
            );
        }
        assert!(!underrun);
        assert_eq!(ring.xruns.load(Ordering::Relaxed), 1);
        assert_eq!(LWASAPI_RING_FRAMES, 32_768);
    }
}

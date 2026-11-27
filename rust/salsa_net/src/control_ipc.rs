// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![deny(unsafe_code)]

use core::mem::size_of;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use crate::c_abi::{
    c_str_to_string, copy_str_to_c, PodBytes, LasapiBtCodecCfg, LasapiDeviceList,
    LasapiEngineStatus, LasapiEqConfig, LasapiHwAnalogCfg, LasapiIpcReq, LasapiIpcResp,
    LasapiMicApoCfg, LasapiMidiEvent, LasapiParamEqPayload, E_FAIL, E_INVALIDARG,
    LASAPI_BT_CONN_CONNECT, LASAPI_BT_CONN_DISCONNECT, LASAPI_BT_CONN_QUERY,
    LASAPI_BT_CONN_RESET, LASAPI_BT_CONN_SET_AUTOCONN, LASAPI_BT_CONN_SET_OPTIONAL,
    LASAPI_CFG_FLAG_CAPTURE_RATE,
    LASAPI_CMD_BT_CONNECTION, LASAPI_CMD_CONTROL_V2, LASAPI_CMD_GET_STATUS,
    LASAPI_CMD_INJECT_MIDI, LASAPI_CMD_LIST_DEVICES, LASAPI_CMD_RELOAD_CONFIG,
    LASAPI_CMD_SAVE_CONFIG, LASAPI_CMD_SET_BT_CODEC, LASAPI_CMD_SET_DEVICES,
    LASAPI_CMD_SET_ENGINE_CONFIG, LASAPI_CMD_SET_EQ, LASAPI_CMD_SET_HW_ANALOG,
    LASAPI_CMD_SET_MIC_APO, LASAPI_CMD_SET_PARAM_EQ, LASAPI_CMD_SET_SESSION_CONTROL,
    LASAPI_CMD_SET_VOLUME, S_OK,
};
use crate::unix_sys::{
    audiodg_sock_path, connect_unix_stream_verified_with_timeout, current_pid,
};

pub const MAX_CONTROL_V2_FRAME: usize = 65_536;
pub const DEFAULT_CONTROL_TIMEOUT: Duration = Duration::from_millis(2_500);
pub const FAST_CONTROL_TIMEOUT: Duration = Duration::from_millis(350);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioDir {
    Playback,
    Capture,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteOp {
    Keep,
    Unmute,
    Mute,
    Toggle,
}

impl MuteOp {
    pub fn from_ui_mode(mode: i32) -> Self {
        match mode {
            0 => Self::Unmute,
            1 => Self::Mute,
            2 => Self::Toggle,
            _ => Self::Keep,
        }
    }

    pub fn from_legacy_share_mode(share_mode: u32) -> Self {
        match share_mode {
            1 => Self::Unmute,
            2 => Self::Mute,
            3 => Self::Toggle,
            _ => Self::Keep,
        }
    }

    pub fn to_legacy_flag(self) -> i32 {
        match self {
            Self::Keep => -1,
            Self::Unmute => 0,
            Self::Mute => 1,
            Self::Toggle => 2,
        }
    }

    fn to_u8(self) -> u8 {
        match self {
            Self::Keep => 0,
            Self::Unmute => 1,
            Self::Mute => 2,
            Self::Toggle => 3,
        }
    }

    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Keep),
            1 => Some(Self::Unmute),
            2 => Some(Self::Mute),
            3 => Some(Self::Toggle),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionTarget {
    NativeShm(u32),
    PulseStream(u32),
}

impl SessionTarget {
    pub fn from_combined_id(id: u32) -> Self {
        if id < 10_000 {
            Self::NativeShm(id)
        } else {
            Self::PulseStream(id)
        }
    }

    pub fn raw_id(self) -> u32 {
        match self {
            Self::NativeShm(id) | Self::PulseStream(id) => id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BtConnOp {
    Query,
    SetOptional,
    SetAutoConn,
    Connect,
    Disconnect,
    Reset,
}

impl BtConnOp {
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            LASAPI_BT_CONN_QUERY => Some(Self::Query),
            LASAPI_BT_CONN_SET_OPTIONAL => Some(Self::SetOptional),
            LASAPI_BT_CONN_SET_AUTOCONN => Some(Self::SetAutoConn),
            LASAPI_BT_CONN_CONNECT => Some(Self::Connect),
            LASAPI_BT_CONN_DISCONNECT => Some(Self::Disconnect),
            LASAPI_BT_CONN_RESET => Some(Self::Reset),
            _ => None,
        }
    }

    pub fn to_u32(self) -> u32 {
        match self {
            Self::Query => LASAPI_BT_CONN_QUERY,
            Self::SetOptional => LASAPI_BT_CONN_SET_OPTIONAL,
            Self::SetAutoConn => LASAPI_BT_CONN_SET_AUTOCONN,
            Self::Connect => LASAPI_BT_CONN_CONNECT,
            Self::Disconnect => LASAPI_BT_CONN_DISCONNECT,
            Self::Reset => LASAPI_BT_CONN_RESET,
        }
    }
}

#[derive(Clone)]
pub enum ControlRequest {
    GetStatus,
    ListDevices,
    SetDevices {
        playback_id: Option<String>,
        capture_id: Option<String>,
    },
    SetMasterVolume {
        dir: AudioDir,
        volume_pct: Option<u32>,
        mute: MuteOp,
        client_tag: String,
    },
    SetSessionVolume {
        session_id: u32,
        volume_milli: u32,
    },
    SetSessionControl {
        target: SessionTarget,
        volume_milli: Option<u32>,
        mute: MuteOp,
        route_device: Option<String>,
    },
    SetPlaybackClock {
        sample_rate: u32,
        period_frames: u32,
        format_bits: u32,
    },
    SetCaptureClock {
        sample_rate: u32,
    },
    SetDynamicEq(Box<LasapiEqConfig>),
    SetParametricEq(Box<LasapiParamEqPayload>),
    SetMicApo(LasapiMicApoCfg),
    SetBtCodec(LasapiBtCodecCfg),
    SetHwAnalog(LasapiHwAnalogCfg),
    BtConnection {
        op: BtConnOp,
        mac: String,
    },
    InjectMidi(LasapiMidiEvent),
    SaveConfig,
    ReloadConfig,
}

#[derive(Clone)]
pub enum ControlPayload {
    Empty,
    Status {
        status: Box<LasapiEngineStatus>,
        active_capture_rate_hz: u32,
    },
    Devices(Box<LasapiDeviceList>),
    ClockAck {
        sample_rate: u32,
        channels: u32,
        period_frames: u32,
        buffer_frames: u32,
    },
    BtFlags(u32),
    BtFlagsMsg {
        flags: u32,
        message: String,
    },
}

#[derive(Clone)]
pub struct ControlResponse {
    pub hr: i32,
    pub payload: ControlPayload,
}

impl ControlResponse {
    pub fn ok() -> Self {
        Self {
            hr: S_OK,
            payload: ControlPayload::Empty,
        }
    }

    pub fn err(hr: i32) -> Self {
        Self {
            hr,
            payload: ControlPayload::Empty,
        }
    }

    pub fn invalid() -> Self {
        Self::err(E_INVALIDARG)
    }
}

fn push_opt_str(out: &mut Vec<u8>, opt: Option<&str>) {
    match opt {
        None => out.push(0),
        Some(s) => {
            out.push(1);
            push_str(out, s);
        }
    }
}

fn push_str(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(u16::MAX as usize);
    out.extend_from_slice(&(len as u16).to_le_bytes());
    out.extend_from_slice(&bytes[..len]);
}

fn push_opt_u32(out: &mut Vec<u8>, opt: Option<u32>) {
    match opt {
        None => out.push(0),
        Some(v) => {
            out.push(1);
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        if end > self.buf.len() {
            return None;
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Some(s)
    }

    fn take_u8(&mut self) -> Option<u8> {
        self.take(1).map(|s| s[0])
    }

    fn take_u16(&mut self) -> Option<u16> {
        let s = self.take(2)?;
        Some(u16::from_le_bytes([s[0], s[1]]))
    }

    fn take_u32(&mut self) -> Option<u32> {
        let s = self.take(4)?;
        Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn take_i32(&mut self) -> Option<i32> {
        let s = self.take(4)?;
        Some(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn take_str(&mut self) -> Option<String> {
        let len = usize::from(self.take_u16()?);
        let bytes = self.take(len)?;
        Some(String::from_utf8_lossy(bytes).into_owned())
    }

    fn take_opt_str(&mut self) -> Option<Option<String>> {
        match self.take_u8()? {
            0 => Some(None),
            1 => self.take_str().map(Some),
            _ => None,
        }
    }

    fn take_opt_u32(&mut self) -> Option<Option<u32>> {
        match self.take_u8()? {
            0 => Some(None),
            1 => self.take_u32().map(Some),
            _ => None,
        }
    }

    fn take_pod<T: PodBytes>(&mut self) -> Option<T> {
        let bytes = self.take(size_of::<T>())?;
        T::from_bytes(bytes)
    }
}

pub fn encode_request(req: &ControlRequest) -> Vec<u8> {
    let mut out = Vec::with_capacity(128);
    match req {
        ControlRequest::GetStatus => out.push(1),
        ControlRequest::ListDevices => out.push(2),
        ControlRequest::SetDevices {
            playback_id,
            capture_id,
        } => {
            out.push(3);
            push_opt_str(&mut out, playback_id.as_deref());
            push_opt_str(&mut out, capture_id.as_deref());
        }
        ControlRequest::SetMasterVolume {
            dir,
            volume_pct,
            mute,
            client_tag,
        } => {
            out.push(4);
            out.push(match dir {
                AudioDir::Playback => 0,
                AudioDir::Capture => 1,
            });
            push_opt_u32(&mut out, *volume_pct);
            out.push(mute.to_u8());
            push_str(&mut out, client_tag);
        }
        ControlRequest::SetSessionVolume {
            session_id,
            volume_milli,
        } => {
            out.push(5);
            out.extend_from_slice(&session_id.to_le_bytes());
            out.extend_from_slice(&volume_milli.to_le_bytes());
        }
        ControlRequest::SetSessionControl {
            target,
            volume_milli,
            mute,
            route_device,
        } => {
            out.push(6);
            out.extend_from_slice(&target.raw_id().to_le_bytes());
            push_opt_u32(&mut out, *volume_milli);
            out.push(mute.to_u8());
            push_opt_str(&mut out, route_device.as_deref());
        }
        ControlRequest::SetPlaybackClock {
            sample_rate,
            period_frames,
            format_bits,
        } => {
            out.push(7);
            out.extend_from_slice(&sample_rate.to_le_bytes());
            out.extend_from_slice(&period_frames.to_le_bytes());
            out.extend_from_slice(&format_bits.to_le_bytes());
        }
        ControlRequest::SetCaptureClock { sample_rate } => {
            out.push(8);
            out.extend_from_slice(&sample_rate.to_le_bytes());
        }
        ControlRequest::SetDynamicEq(eq) => {
            out.push(9);
            out.extend_from_slice(&eq.to_bytes());
        }
        ControlRequest::SetParametricEq(pay) => {
            out.push(10);
            out.extend_from_slice(&pay.to_bytes());
        }
        ControlRequest::SetMicApo(cfg) => {
            out.push(11);
            out.extend_from_slice(&cfg.to_bytes());
        }
        ControlRequest::SetBtCodec(cfg) => {
            out.push(12);
            out.extend_from_slice(&cfg.to_bytes());
        }
        ControlRequest::SetHwAnalog(cfg) => {
            out.push(13);
            out.extend_from_slice(&cfg.to_bytes());
        }
        ControlRequest::BtConnection { op, mac } => {
            out.push(14);
            out.extend_from_slice(&op.to_u32().to_le_bytes());
            push_str(&mut out, mac);
        }
        ControlRequest::InjectMidi(ev) => {
            out.push(15);
            out.extend_from_slice(&ev.to_bytes());
        }
        ControlRequest::SaveConfig => out.push(16),
        ControlRequest::ReloadConfig => out.push(17),
    }
    out
}

pub fn decode_request(raw: &[u8]) -> Option<ControlRequest> {
    let mut c = Cursor::new(raw);
    let tag = c.take_u8()?;
    let req = match tag {
        1 => ControlRequest::GetStatus,
        2 => ControlRequest::ListDevices,
        3 => ControlRequest::SetDevices {
            playback_id: c.take_opt_str()?,
            capture_id: c.take_opt_str()?,
        },
        4 => {
            let dir = match c.take_u8()? {
                0 => AudioDir::Playback,
                1 => AudioDir::Capture,
                _ => return None,
            };
            let volume_pct = c.take_opt_u32()?;
            let mute = MuteOp::from_u8(c.take_u8()?)?;
            let client_tag = c.take_str()?;
            ControlRequest::SetMasterVolume {
                dir,
                volume_pct,
                mute,
                client_tag,
            }
        }
        5 => ControlRequest::SetSessionVolume {
            session_id: c.take_u32()?,
            volume_milli: c.take_u32()?,
        },
        6 => {
            let id = c.take_u32()?;
            let volume_milli = c.take_opt_u32()?;
            let mute = MuteOp::from_u8(c.take_u8()?)?;
            let route_device = c.take_opt_str()?;
            ControlRequest::SetSessionControl {
                target: SessionTarget::from_combined_id(id),
                volume_milli,
                mute,
                route_device,
            }
        }
        7 => ControlRequest::SetPlaybackClock {
            sample_rate: c.take_u32()?,
            period_frames: c.take_u32()?,
            format_bits: c.take_u32()?,
        },
        8 => ControlRequest::SetCaptureClock {
            sample_rate: c.take_u32()?,
        },
        9 => ControlRequest::SetDynamicEq(Box::new(c.take_pod()?)),
        10 => ControlRequest::SetParametricEq(Box::new(c.take_pod()?)),
        11 => ControlRequest::SetMicApo(c.take_pod()?),
        12 => ControlRequest::SetBtCodec(c.take_pod()?),
        13 => ControlRequest::SetHwAnalog(c.take_pod()?),
        14 => ControlRequest::BtConnection {
            op: BtConnOp::from_u32(c.take_u32()?)?,
            mac: c.take_str()?,
        },
        15 => ControlRequest::InjectMidi(c.take_pod()?),
        16 => ControlRequest::SaveConfig,
        17 => ControlRequest::ReloadConfig,
        _ => return None,
    };
    (c.pos == raw.len()).then_some(req)
}

pub fn encode_response(resp: &ControlResponse) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&resp.hr.to_le_bytes());
    match &resp.payload {
        ControlPayload::Empty => out.push(0),
        ControlPayload::Status {
            status,
            active_capture_rate_hz,
        } => {
            out.push(1);
            out.extend_from_slice(&active_capture_rate_hz.to_le_bytes());
            out.extend_from_slice(&status.to_bytes());
        }
        ControlPayload::Devices(list) => {
            out.push(2);
            out.extend_from_slice(&list.to_bytes());
        }
        ControlPayload::ClockAck {
            sample_rate,
            channels,
            period_frames,
            buffer_frames,
        } => {
            out.push(3);
            out.extend_from_slice(&sample_rate.to_le_bytes());
            out.extend_from_slice(&channels.to_le_bytes());
            out.extend_from_slice(&period_frames.to_le_bytes());
            out.extend_from_slice(&buffer_frames.to_le_bytes());
        }
        ControlPayload::BtFlags(flags) => {
            out.push(4);
            out.extend_from_slice(&flags.to_le_bytes());
        }
        ControlPayload::BtFlagsMsg { flags, message } => {
            out.push(5);
            out.extend_from_slice(&flags.to_le_bytes());
            push_str(&mut out, message);
        }
    }
    out
}

pub fn decode_response(raw: &[u8]) -> Option<ControlResponse> {
    let mut c = Cursor::new(raw);
    let hr = c.take_i32()?;
    let payload = match c.take_u8()? {
        0 => ControlPayload::Empty,
        1 => {
            let active_capture_rate_hz = c.take_u32()?;
            let status = Box::new(c.take_pod()?);
            ControlPayload::Status {
                status,
                active_capture_rate_hz,
            }
        }
        2 => ControlPayload::Devices(Box::new(c.take_pod()?)),
        3 => ControlPayload::ClockAck {
            sample_rate: c.take_u32()?,
            channels: c.take_u32()?,
            period_frames: c.take_u32()?,
            buffer_frames: c.take_u32()?,
        },
        4 => ControlPayload::BtFlags(c.take_u32()?),
        5 => ControlPayload::BtFlagsMsg {
            flags: c.take_u32()?,
            message: c.take_str()?,
        },
        _ => return None,
    };
    (c.pos == raw.len()).then_some(ControlResponse { hr, payload })
}

pub fn write_response_frame(stream: &mut UnixStream, resp: &ControlResponse) -> std::io::Result<()> {
    let bytes = encode_response(resp);
    let len_u32 = u32::try_from(bytes.len()).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "response frame too large")
    })?;
    stream.write_all(&len_u32.to_le_bytes())?;
    stream.write_all(&bytes)
}

pub fn read_response_frame(stream: &mut UnixStream) -> Option<ControlResponse> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).ok()?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len == 0 || len > MAX_CONTROL_V2_FRAME {
        return None;
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).ok()?;
    decode_response(&buf)
}

pub struct ControlClient;

impl ControlClient {
    pub fn send_over_stream(stream: &mut UnixStream, req: &ControlRequest) -> Result<ControlResponse, i32> {
        let payload = encode_request(req);
        if payload.len() > MAX_CONTROL_V2_FRAME {
            return Err(E_INVALIDARG);
        }
        let mut hdr = LasapiIpcReq::from_bytes(&[0u8; size_of::<LasapiIpcReq>()])
            .ok_or(E_FAIL)?;
        hdr.cmd = LASAPI_CMD_CONTROL_V2;
        hdr.client_pid = current_pid();
        hdr.requested_buffer_frames = payload.len() as u32;
        if let ControlRequest::SetMasterVolume { client_tag, .. } = req {
            if !client_tag.is_empty() {
                copy_str_to_c(&mut hdr.client_name, client_tag);
            }
        }
        if stream.write_all(&hdr.to_bytes()).is_err() || stream.write_all(&payload).is_err() {
            return Err(E_FAIL);
        }
        read_response_frame(stream).ok_or(E_FAIL)
    }

    pub fn send_with_timeout(
        sock_path: &str,
        req: &ControlRequest,
        timeout: Duration,
    ) -> Result<ControlResponse, i32> {
        let mut stream = connect_unix_stream_verified_with_timeout(sock_path, timeout).ok_or(E_FAIL)?;
        Self::send_over_stream(&mut stream, req)
    }

    pub fn send_if_reachable_with_timeout(
        sock_path: &str,
        req: &ControlRequest,
        timeout: Duration,
    ) -> Option<Result<ControlResponse, i32>> {
        let mut stream = connect_unix_stream_verified_with_timeout(sock_path, timeout)?;
        Some(Self::send_over_stream(&mut stream, req))
    }

    pub fn send(req: &ControlRequest) -> Result<ControlResponse, i32> {
        Self::send_with_timeout(&audiodg_sock_path(), req, DEFAULT_CONTROL_TIMEOUT)
    }

    pub fn send_fast(req: &ControlRequest) -> Result<ControlResponse, i32> {
        Self::send_with_timeout(&audiodg_sock_path(), req, FAST_CONTROL_TIMEOUT)
    }
}

pub fn legacy_ipc_req_to_control(req: &LasapiIpcReq, payload: &[u8]) -> Option<ControlRequest> {
    match req.cmd {
        LASAPI_CMD_GET_STATUS => Some(ControlRequest::GetStatus),
        LASAPI_CMD_LIST_DEVICES => Some(ControlRequest::ListDevices),
        LASAPI_CMD_SET_DEVICES => {
            let pb_s = c_str_to_string(&req.target_device);
            let cap_s = c_str_to_string(&req.target_capture_device);
            Some(ControlRequest::SetDevices {
                playback_id: (!pb_s.is_empty()).then_some(pb_s),
                capture_id: (!cap_s.is_empty()).then_some(cap_s),
            })
        }
        LASAPI_CMD_SET_VOLUME => {
            if req.target_session_id == 0xFFFF_FFFF || req.target_session_id == 0xFFFF_FFFE {
                let dir = if req.target_session_id == 0xFFFF_FFFF {
                    AudioDir::Playback
                } else {
                    AudioDir::Capture
                };
                let volume_pct = (req.volume_milli <= 1500).then_some(req.volume_milli / 10);
                Some(ControlRequest::SetMasterVolume {
                    dir,
                    volume_pct,
                    mute: MuteOp::from_legacy_share_mode(req.share_mode),
                    client_tag: c_str_to_string(&req.client_name),
                })
            } else {
                Some(ControlRequest::SetSessionVolume {
                    session_id: req.target_session_id,
                    volume_milli: req.volume_milli,
                })
            }
        }
        LASAPI_CMD_SET_SESSION_CONTROL => {
            let route = c_str_to_string(&req.target_device);
            Some(ControlRequest::SetSessionControl {
                target: SessionTarget::from_combined_id(req.target_session_id),
                volume_milli: (req.volume_milli <= 1500).then_some(req.volume_milli),
                mute: MuteOp::from_legacy_share_mode(req.share_mode),
                route_device: (!route.is_empty()).then_some(route),
            })
        }
        LASAPI_CMD_INJECT_MIDI => Some(ControlRequest::InjectMidi(req.midi_ev)),
        LASAPI_CMD_SET_ENGINE_CONFIG => {
            if req.stream_flags == LASAPI_CFG_FLAG_CAPTURE_RATE {
                Some(ControlRequest::SetCaptureClock {
                    sample_rate: req.sample_rate,
                })
            } else {
                Some(ControlRequest::SetPlaybackClock {
                    sample_rate: req.sample_rate,
                    period_frames: req.requested_period_frames,
                    format_bits: req.format,
                })
            }
        }
        LASAPI_CMD_SET_EQ => Some(ControlRequest::SetDynamicEq(Box::new(req.eq_cfg))),
        LASAPI_CMD_SET_PARAM_EQ => Some(ControlRequest::SetParametricEq(Box::new(
            LasapiParamEqPayload::from_bytes(payload)?,
        ))),
        LASAPI_CMD_SET_MIC_APO => Some(ControlRequest::SetMicApo(
            LasapiMicApoCfg::from_bytes(payload)?,
        )),
        LASAPI_CMD_SET_BT_CODEC => Some(ControlRequest::SetBtCodec(
            LasapiBtCodecCfg::from_bytes(payload)?,
        )),
        LASAPI_CMD_SET_HW_ANALOG => Some(ControlRequest::SetHwAnalog(
            LasapiHwAnalogCfg::from_bytes(payload)?,
        )),
        LASAPI_CMD_BT_CONNECTION => Some(ControlRequest::BtConnection {
            op: BtConnOp::from_u32(req.requested_period_frames)?,
            mac: c_str_to_string(&req.target_device),
        }),
        LASAPI_CMD_SAVE_CONFIG => Some(ControlRequest::SaveConfig),
        LASAPI_CMD_RELOAD_CONFIG => Some(ControlRequest::ReloadConfig),
        _ => None,
    }
}

pub fn control_resp_to_legacy(resp: &ControlResponse) -> LasapiIpcResp {
    let mut out = LasapiIpcResp {
        hr: resp.hr,
        session_id: 0,
        actual_sample_rate: 0,
        actual_channels: 0,
        actual_buffer_frames: 0,
        actual_period_frames: 0,
    };
    match resp.payload {
        ControlPayload::ClockAck {
            sample_rate,
            channels,
            period_frames,
            buffer_frames,
        } => {
            out.actual_sample_rate = sample_rate;
            out.actual_channels = channels;
            out.actual_period_frames = period_frames;
            out.actual_buffer_frames = buffer_frames;
        }
        ControlPayload::BtFlags(flags) | ControlPayload::BtFlagsMsg { flags, .. } => {
            out.session_id = flags;
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{default_eq_cfg, default_param_eq_ext};

    #[test]
    fn control_request_and_response_roundtrip_all_variants() {
        let eq = default_eq_cfg();
        let ext = default_param_eq_ext();
        let reqs = vec![
            ControlRequest::GetStatus,
            ControlRequest::ListDevices,
            ControlRequest::SetDevices {
                playback_id: Some("hw:1,0".into()),
                capture_id: None,
            },
            ControlRequest::SetMasterVolume {
                dir: AudioDir::Capture,
                volume_pct: Some(85),
                mute: MuteOp::Toggle,
                client_tag: "lasapi-tray".into(),
            },
            ControlRequest::SetSessionControl {
                target: SessionTarget::PulseStream(10_042),
                volume_milli: Some(900),
                mute: MuteOp::Mute,
                route_device: Some("bluez:AA:BB:CC:DD:EE:FF".into()),
            },
            ControlRequest::SetPlaybackClock {
                sample_rate: 96_000,
                period_frames: 128,
                format_bits: 32,
            },
            ControlRequest::SetCaptureClock {
                sample_rate: 48_000,
            },
            ControlRequest::SetParametricEq(Box::new(LasapiParamEqPayload { eq, ext })),
            ControlRequest::BtConnection {
                op: BtConnOp::Connect,
                mac: "AA:BB:CC:DD:EE:FF".into(),
            },
            ControlRequest::BtConnection {
                op: BtConnOp::Reset,
                mac: "11:22:33:44:55:66".into(),
            },
            ControlRequest::SaveConfig,
            ControlRequest::ReloadConfig,
        ];

        for req in &reqs {
            let bytes = encode_request(req);
            assert!(bytes.len() <= MAX_CONTROL_V2_FRAME);
            let dec = decode_request(&bytes).expect("decode_request");
            assert_eq!(encode_request(&dec), bytes);
        }

        let mut st = LasapiEngineStatus::from_bytes(&[0u8; size_of::<LasapiEngineStatus>()])
            .unwrap();
        st.hw_sample_rate = 192_000;
        st.hw_analog_info.reserved = 48_000;
        assert_eq!(st.effective_capture_rate_hz(), 48_000);

        let resp = ControlResponse {
            hr: S_OK,
            payload: ControlPayload::Status {
                status: Box::new(st),
                active_capture_rate_hz: 48_000,
            },
        };
        let rbytes = encode_response(&resp);
        let rdec = decode_response(&rbytes).expect("decode_response");
        assert_eq!(rdec.hr, S_OK);
        match rdec.payload {
            ControlPayload::Status {
                active_capture_rate_hz,
                status,
            } => {
                assert_eq!(active_capture_rate_hz, 48_000);
                assert_eq!(status.hw_sample_rate, 192_000);
            }
            _ => panic!("expected Status payload"),
        }

        let bt_msg = ControlResponse {
            hr: S_OK,
            payload: ControlPayload::BtFlagsMsg {
                flags: 0x3,
                message: "Headset reconnected cleanly".into(),
            },
        };
        let bdec = decode_response(&encode_response(&bt_msg)).expect("decode BtFlagsMsg");
        match bdec.payload {
            ControlPayload::BtFlagsMsg { flags, message } => {
                assert_eq!(flags, 0x3);
                assert_eq!(message, "Headset reconnected cleanly");
            }
            _ => panic!("expected BtFlagsMsg payload"),
        }
    }

    #[test]
    fn control_client_unix_pair_rejects_oversized_response_frame() {
        let (mut cli, mut srv) = UnixStream::pair().unwrap();
        let h = std::thread::spawn(move || {
            let mut hdr_buf = [0u8; size_of::<LasapiIpcReq>()];
            srv.read_exact(&mut hdr_buf).unwrap();
            let hdr = LasapiIpcReq::from_bytes(&hdr_buf).unwrap();
            assert_eq!(hdr.cmd, LASAPI_CMD_CONTROL_V2);
            let mut pay = vec![0u8; hdr.requested_buffer_frames as usize];
            srv.read_exact(&mut pay).unwrap();
            let req = decode_request(&pay).unwrap();
            assert!(matches!(req, ControlRequest::SaveConfig));
            write_response_frame(&mut srv, &ControlResponse::ok()).unwrap();
        });
        let resp = ControlClient::send_over_stream(&mut cli, &ControlRequest::SaveConfig)
            .expect("send_over_stream");
        assert_eq!(resp.hr, S_OK);
        h.join().unwrap();

        let (mut cli2, mut srv2) = UnixStream::pair().unwrap();
        let h2 = std::thread::spawn(move || {
            let mut hdr_buf = [0u8; size_of::<LasapiIpcReq>()];
            srv2.read_exact(&mut hdr_buf).unwrap();
            let hdr = LasapiIpcReq::from_bytes(&hdr_buf).unwrap();
            let mut pay = vec![0u8; hdr.requested_buffer_frames as usize];
            srv2.read_exact(&mut pay).unwrap();
            let huge = (MAX_CONTROL_V2_FRAME as u32) + 1;
            srv2.write_all(&huge.to_le_bytes()).unwrap();
        });
        let err = ControlClient::send_over_stream(&mut cli2, &ControlRequest::SaveConfig)
            .err()
            .expect("oversized frame should fail");
        assert_eq!(err, E_FAIL);
        h2.join().unwrap();
    }
}

// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![deny(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use crate::gio_dbus::dbus_session_call_string;
use crate::x11_util::plasma_tray_panel_info;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn is_valid(self) -> bool {
        self.w > 0 && self.h > 0
    }

    pub fn contains(self, px: i32, py: i32) -> bool {
        self.is_valid()
            && px >= self.x
            && px < self.x + self.w
            && py >= self.y
            && py < self.y + self.h
    }

    pub fn center(self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    pub fn right_exclusive(self) -> i32 {
        self.x + self.w
    }

    pub fn bottom_exclusive(self) -> i32 {
        self.y + self.h
    }
}

#[derive(Debug, Clone)]
pub struct ScreenInfo {
    pub name: String,
    pub geo: Rect,
    pub avail: Rect,
    pub dpr: f64,
}

pub trait CompositorQuery {
    fn kwin_active_output_name(&self) -> Option<String>;
    fn kwin_support_information(&self) -> Option<String>;
    fn plasma_panel_info(&self) -> (i32, i32, i32);
}

pub struct GioCompositorQuery;

impl CompositorQuery for GioCompositorQuery {
    fn kwin_active_output_name(&self) -> Option<String> {
        dbus_session_call_string(
            "org.kde.KWin",
            "/KWin",
            "org.kde.KWin",
            "activeOutputName",
            35,
        )
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    }

    fn kwin_support_information(&self) -> Option<String> {
        dbus_session_call_string(
            "org.kde.KWin",
            "/KWin",
            "org.kde.KWin",
            "supportInformation",
            80,
        )
    }

    fn plasma_panel_info(&self) -> (i32, i32, i32) {
        plasma_tray_panel_info()
    }
}

#[derive(Default)]
struct KwinScaleCache {
    geo_sig: String,
    scales: BTreeMap<String, f64>,
}

static KWIN_SCALE_CACHE: OnceLock<Mutex<KwinScaleCache>> = OnceLock::new();

pub fn parse_kwin_output_scales(support_info: &str) -> BTreeMap<String, f64> {
    let mut parsed = BTreeMap::new();
    let mut cur_name = String::new();
    for raw in support_info.lines() {
        let ln = raw.trim();
        if let Some(rest) = ln.strip_prefix("Name:") {
            cur_name = rest.trim().to_string();
        } else if !cur_name.is_empty() {
            if let Some(rest) = ln.strip_prefix("Scale:") {
                if let Ok(sc) = rest.trim().parse::<f64>() {
                    if sc.is_finite() && (0.5..=8.0).contains(&sc) {
                        parsed.insert(cur_name.to_ascii_lowercase(), sc);
                    }
                }
                cur_name.clear();
            }
        }
    }
    parsed
}

fn build_geo_sig(screens: &[ScreenInfo]) -> String {
    let mut sig = String::new();
    for s in screens {
        sig.push_str(&format!(
            "{}:{},{},{}x{};",
            s.name, s.geo.x, s.geo.y, s.geo.w, s.geo.h
        ));
    }
    sig
}

pub fn query_kwin_output_scales_uncached(query: &dyn CompositorQuery) -> BTreeMap<String, f64> {
    query
        .kwin_support_information()
        .map(|txt| parse_kwin_output_scales(&txt))
        .unwrap_or_default()
}

pub fn query_kwin_output_scales_cached(
    screens: &[ScreenInfo],
    query: &dyn CompositorQuery,
) -> BTreeMap<String, f64> {
    let sig = build_geo_sig(screens);
    let cache_lock = KWIN_SCALE_CACHE.get_or_init(|| Mutex::new(KwinScaleCache::default()));
    {
        let cache = cache_lock.lock().unwrap_or_else(|e| e.into_inner());
        if !sig.is_empty() && sig == cache.geo_sig && !cache.scales.is_empty() {
            return cache.scales.clone();
        }
    }
    let parsed = query_kwin_output_scales_uncached(query);
    let mut cache = cache_lock.lock().unwrap_or_else(|e| e.into_inner());
    if !parsed.is_empty() {
        cache.geo_sig = sig;
        cache.scales = parsed.clone();
        return parsed;
    }
    cache.scales.clone()
}

pub fn effective_wayland_scale(scr: &ScreenInfo, kwin_scales: &BTreeMap<String, f64>) -> f64 {
    let key = scr.name.trim().to_ascii_lowercase();
    if let Some(&sc) = kwin_scales.get(&key) {
        if sc.is_finite() && sc >= 0.5 {
            return sc;
        }
    }
    if scr.dpr.is_finite() && scr.dpr > 1.0 {
        scr.dpr
    } else {
        1.0
    }
}

pub fn try_plasma_mixed_point(
    scr: &ScreenInfo,
    gx: i32,
    gy: i32,
    kwin_scales: &BTreeMap<String, f64>,
) -> Option<(i32, i32)> {
    let sg = scr.geo;
    if sg.w <= 0 || sg.h <= 0 {
        return None;
    }
    let dpr = effective_wayland_scale(scr, kwin_scales);
    let lx = ((f64::from(gx - sg.x)) / dpr).round() as i32;
    let ly = ((f64::from(gy - sg.y)) / dpr).round() as i32;
    if lx >= -6 && lx <= sg.w + 6 && ly >= -6 && ly <= sg.h + 6 {
        let cx = sg.x + lx.clamp(0, (sg.w - 1).max(0));
        let cy = sg.y + ly.clamp(0, (sg.h - 1).max(0));
        Some((cx, cy))
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TrayResolveReq {
    pub raw_x: i32,
    pub raw_y: i32,
    pub has_raw: bool,
    pub cursor_x: i32,
    pub cursor_y: i32,
    pub is_xcb: bool,
    pub is_kde_env: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedTrayTarget {
    pub screen_idx: usize,
    pub qt_x: i32,
    pub qt_y: i32,
}

fn find_kwin_active_screen_idx(
    screens: &[ScreenInfo],
    query: &dyn CompositorQuery,
) -> Option<usize> {
    let active = query.kwin_active_output_name()?;
    let active_trimmed = active.trim();
    if active_trimmed.is_empty() {
        return None;
    }
    screens
        .iter()
        .position(|s| s.name.eq_ignore_ascii_case(active_trimmed))
}

fn find_screen_containing(screens: &[ScreenInfo], px: i32, py: i32) -> Option<usize> {
    screens.iter().position(|s| s.geo.contains(px, py))
}

pub fn resolve_tray_target_with_scales(
    screens: &[ScreenInfo],
    req: &TrayResolveReq,
    kwin_active_idx: Option<usize>,
    kwin_scales: &BTreeMap<String, f64>,
) -> ResolvedTrayTarget {
    if screens.is_empty() {
        return ResolvedTrayTarget {
            screen_idx: 0,
            qt_x: req.cursor_x,
            qt_y: req.cursor_y,
        };
    }

    if !req.has_raw || (req.raw_x == 0 && req.raw_y == 0) {
        let idx = if !req.is_xcb { kwin_active_idx } else { None }
            .or_else(|| find_screen_containing(screens, req.cursor_x, req.cursor_y))
            .unwrap_or(0);
        let (cx, cy) = screens[idx].geo.center();
        return ResolvedTrayTarget {
            screen_idx: idx,
            qt_x: cx,
            qt_y: cy,
        };
    }

    let gx = req.raw_x;
    let gy = req.raw_y;

    if !req.is_xcb {
        let is_kde = kwin_active_idx.is_some() || req.is_kde_env;
        if is_kde {
            if let Some(k_idx) = kwin_active_idx {
                if let Some((px, py)) = try_plasma_mixed_point(&screens[k_idx], gx, gy, kwin_scales)
                {
                    return ResolvedTrayTarget {
                        screen_idx: k_idx,
                        qt_x: px,
                        qt_y: py,
                    };
                }
            }
        } else if let Some(idx) = find_screen_containing(screens, gx, gy) {
            return ResolvedTrayTarget {
                screen_idx: idx,
                qt_x: gx,
                qt_y: gy,
            };
        }

        let mut best_plasma: Option<(usize, i32, i32, i32)> = None;
        for (idx, s) in screens.iter().enumerate() {
            if let Some((px, py)) = try_plasma_mixed_point(s, gx, gy, kwin_scales) {
                let lx = px - s.geo.x;
                let replace = best_plasma.is_none_or(|(_, _, _, best_lx)| lx > best_lx);
                if replace {
                    best_plasma = Some((idx, px, py, lx));
                }
            }
        }
        if let Some((idx, px, py, _)) = best_plasma {
            return ResolvedTrayTarget {
                screen_idx: idx,
                qt_x: px,
                qt_y: py,
            };
        }

        if let Some(idx) = find_screen_containing(screens, gx, gy) {
            return ResolvedTrayTarget {
                screen_idx: idx,
                qt_x: gx,
                qt_y: gy,
            };
        }

        for (idx, s) in screens.iter().enumerate() {
            let dpr = effective_wayland_scale(s, kwin_scales);
            let sx = (f64::from(gx) / dpr).round() as i32;
            let sy = (f64::from(gy) / dpr).round() as i32;
            if s.geo.contains(sx, sy) {
                return ResolvedTrayTarget {
                    screen_idx: idx,
                    qt_x: sx,
                    qt_y: sy,
                };
            }
        }

        return ResolvedTrayTarget {
            screen_idx: kwin_active_idx.unwrap_or(0),
            qt_x: gx,
            qt_y: gy,
        };
    }

    // XCB / XWayland coordinate mapping
    let mut qt_x = gx;
    let mut qt_y = gy;
    let mut matched = false;
    for s in screens {
        let sg = s.geo;
        let dpr = if s.dpr.is_finite() && s.dpr > 1.0 {
            s.dpr
        } else {
            1.0
        };
        let wx = (f64::from(sg.x) / dpr).round() as i32;
        let wy = (f64::from(sg.y) / dpr).round() as i32;
        if wx <= gx && gx < wx + sg.w && wy <= gy && gy < wy + sg.h {
            qt_x = sg.x + (gx - wx);
            qt_y = sg.y + (gy - wy);
            matched = true;
            break;
        }
    }
    if !matched {
        for s in screens {
            let sg = s.geo;
            let dpr = if s.dpr.is_finite() && s.dpr > 1.0 {
                s.dpr
            } else {
                1.0
            };
            let px = (f64::from(sg.x) / dpr).round() * dpr;
            let py = (f64::from(sg.y) / dpr).round() * dpr;
            let pw = f64::from(sg.w) * dpr;
            let ph = f64::from(sg.h) * dpr;
            let gfx = f64::from(gx);
            let gfy = f64::from(gy);
            if px <= gfx && gfx < px + pw && py <= gfy && gfy < py + ph {
                qt_x = sg.x + ((gfx - px) / dpr).round() as i32;
                qt_y = sg.y + ((gfy - py) / dpr).round() as i32;
                break;
            }
        }
    }
    let idx = find_screen_containing(screens, qt_x, qt_y)
        .or_else(|| find_screen_containing(screens, req.cursor_x, req.cursor_y))
        .unwrap_or(0);
    ResolvedTrayTarget {
        screen_idx: idx,
        qt_x,
        qt_y,
    }
}

pub fn resolve_tray_target(
    screens: &[ScreenInfo],
    req: &TrayResolveReq,
    query: &dyn CompositorQuery,
) -> ResolvedTrayTarget {
    let kwin_active = if !req.is_xcb {
        find_kwin_active_screen_idx(screens, query)
    } else {
        None
    };
    let scales = if !req.is_xcb {
        query_kwin_output_scales_cached(screens, query)
    } else {
        BTreeMap::new()
    };
    resolve_tray_target_with_scales(screens, req, kwin_active, &scales)
}

#[derive(Debug, Clone, Copy)]
pub struct TrayPopupReq {
    pub resolve: TrayResolveReq,
    pub pop_w: i32,
    pub pop_h: i32,
    pub edge_pad: i32,
    pub is_layer_shell: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayPopupPlan {
    pub screen_idx: usize,
    pub qt_x: i32,
    pub qt_y: i32,
    pub avail: Rect,
    pub screen_geo: Rect,
    pub panel_loc: i32,
    pub panel_clearance: i32,
    pub logical_pad: i32,
    pub anchor_raw_local_offset: i32,
    pub anchor_bottom: i32,
    pub popup_x: i32,
    pub popup_y: i32,
    pub layer_anchor_top: bool,
    pub layer_anchor_bottom: bool,
    pub layer_anchor_left: bool,
    pub layer_anchor_right: bool,
    pub margin_left: i32,
    pub margin_top: i32,
    pub margin_right: i32,
    pub margin_bottom: i32,
}

fn default_screen() -> ScreenInfo {
    ScreenInfo {
        name: String::new(),
        geo: Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        },
        avail: Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1032,
        },
        dpr: 1.0,
    }
}

pub fn effective_avail_rect(scr: &ScreenInfo) -> Rect {
    if scr.avail.w >= 200 && scr.avail.h >= 200 {
        scr.avail
    } else if scr.geo.w > 0 && scr.geo.h > 0 {
        scr.geo
    } else {
        Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        }
    }
}

pub fn compute_tray_popup_plan_with_scales(
    screens: &[ScreenInfo],
    req: &TrayPopupReq,
    kwin_active_idx: Option<usize>,
    kwin_scales: &BTreeMap<String, f64>,
    panel_info: (i32, i32, i32),
) -> TrayPopupPlan {
    let tgt = resolve_tray_target_with_scales(screens, &req.resolve, kwin_active_idx, kwin_scales);
    let fallback = default_screen();
    let scr = screens.get(tgt.screen_idx).unwrap_or(&fallback);
    let sg = if scr.geo.is_valid() {
        scr.geo
    } else {
        fallback.geo
    };
    let avail = effective_avail_rect(scr);
    let edge = req.edge_pad.max(0);

    let (loc, clearance, right_pad) = panel_info;
    let dpr = if scr.dpr.is_finite() && scr.dpr > 1.0 {
        scr.dpr
    } else {
        1.0
    };
    let logical_pad = if req.resolve.is_xcb {
        (f64::from(right_pad) / dpr).round() as i32
    } else {
        right_pad
    };

    let pop_w = req.pop_w.max(1);
    let pop_h = req.pop_h.max(1);
    let has_raw = req.resolve.has_raw;

    let mut anchor_raw_local_offset = -1;
    let (
        layer_anchor_top,
        layer_anchor_bottom,
        layer_anchor_left,
        layer_anchor_right,
        margin_left,
        margin_top,
        margin_right,
        margin_bottom,
    ) = if loc == 5 || loc == 6 {
        let is_left = loc == 5;
        let local_y = if has_raw {
            anchor_raw_local_offset = tgt.qt_y - sg.y;
            let max_y = (sg.h - pop_h - edge).max(edge);
            (anchor_raw_local_offset - pop_h / 2).clamp(edge, max_y)
        } else {
            (sg.h - pop_h - edge * 2).max(edge)
        };
        (true, false, is_left, !is_left, edge, local_y, edge, edge)
    } else {
        let is_top = loc == 3;
        if has_raw {
            anchor_raw_local_offset = tgt.qt_x - sg.x;
            let max_x = (sg.w - pop_w - edge).max(edge);
            let local_x = (anchor_raw_local_offset - pop_w / 2).clamp(edge, max_x);
            (is_top, !is_top, true, false, local_x, edge, 0, edge)
        } else {
            (
                is_top,
                !is_top,
                false,
                true,
                0,
                edge,
                logical_pad.max(16),
                edge,
            )
        }
    };

    let min_px = avail.x + edge;
    let max_px = (avail.right_exclusive() - pop_w - edge).max(min_px);
    let raw_px = if has_raw {
        tgt.qt_x - pop_w / 2
    } else {
        avail.right_exclusive() - pop_w - logical_pad.max(16)
    };
    let popup_x = raw_px.clamp(min_px, max_px);

    let min_py = avail.y + edge;
    let max_py = (avail.bottom_exclusive() - pop_h - edge).max(min_py);
    let raw_py = if loc == 3 {
        avail.y + edge
    } else {
        avail.bottom_exclusive() - pop_h - edge
    };
    let popup_y = raw_py.clamp(min_py, max_py);
    let anchor_bottom = avail.bottom_exclusive() - edge;

    TrayPopupPlan {
        screen_idx: tgt.screen_idx,
        qt_x: tgt.qt_x,
        qt_y: tgt.qt_y,
        avail,
        screen_geo: sg,
        panel_loc: loc,
        panel_clearance: clearance,
        logical_pad,
        anchor_raw_local_offset,
        anchor_bottom,
        popup_x,
        popup_y,
        layer_anchor_top,
        layer_anchor_bottom,
        layer_anchor_left,
        layer_anchor_right,
        margin_left,
        margin_top,
        margin_right,
        margin_bottom,
    }
}

pub fn compute_tray_popup_plan(
    screens: &[ScreenInfo],
    req: &TrayPopupReq,
    query: &dyn CompositorQuery,
) -> TrayPopupPlan {
    let kwin_active = if !req.resolve.is_xcb {
        find_kwin_active_screen_idx(screens, query)
    } else {
        None
    };
    let scales = if !req.resolve.is_xcb {
        query_kwin_output_scales_cached(screens, query)
    } else {
        BTreeMap::new()
    };
    let panel = query.plasma_panel_info();
    compute_tray_popup_plan_with_scales(screens, req, kwin_active, &scales, panel)
}

#[derive(Debug, Clone, Copy)]
pub struct TrayResizeReq {
    pub screen_geo: Rect,
    pub avail: Rect,
    pub panel_loc: i32,
    pub anchor_raw_local_offset: i32,
    pub anchor_bottom: i32,
    pub cur_x: i32,
    pub cur_y: i32,
    pub pop_w: i32,
    pub pop_h: i32,
    pub edge_pad: i32,
    pub is_layer_shell: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayResizePlan {
    pub update_margins: bool,
    pub margin_left: i32,
    pub margin_top: i32,
    pub margin_right: i32,
    pub margin_bottom: i32,
    pub move_floating: bool,
    pub new_x: i32,
    pub new_y: i32,
}

pub fn compute_tray_resize_plan(req: &TrayResizeReq) -> TrayResizePlan {
    let edge = req.edge_pad.max(0);
    let pop_w = req.pop_w.max(1);
    let pop_h = req.pop_h.max(1);

    if req.is_layer_shell {
        if req.anchor_raw_local_offset >= 0 {
            if req.panel_loc == 5 || req.panel_loc == 6 {
                let max_y = (req.screen_geo.h - pop_h - edge).max(edge);
                let local_y = (req.anchor_raw_local_offset - pop_h / 2).clamp(edge, max_y);
                return TrayResizePlan {
                    update_margins: true,
                    margin_left: edge,
                    margin_top: local_y,
                    margin_right: edge,
                    margin_bottom: edge,
                    move_floating: false,
                    new_x: req.cur_x,
                    new_y: req.cur_y,
                };
            }
            let max_x = (req.screen_geo.w - pop_w - edge).max(edge);
            let local_x = (req.anchor_raw_local_offset - pop_w / 2).clamp(edge, max_x);
            return TrayResizePlan {
                update_margins: true,
                margin_left: local_x,
                margin_top: edge,
                margin_right: 0,
                margin_bottom: edge,
                move_floating: false,
                new_x: req.cur_x,
                new_y: req.cur_y,
            };
        }
        return TrayResizePlan {
            update_margins: false,
            margin_left: 0,
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            move_floating: false,
            new_x: req.cur_x,
            new_y: req.cur_y,
        };
    }

    if req.panel_loc != 3 && req.avail.is_valid() {
        let top_lim = req.avail.y + edge;
        let new_y = top_lim.max(req.anchor_bottom - pop_h);
        TrayResizePlan {
            update_margins: false,
            margin_left: 0,
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            move_floating: true,
            new_x: req.cur_x,
            new_y,
        }
    } else {
        TrayResizePlan {
            update_margins: false,
            margin_left: 0,
            margin_top: 0,
            margin_right: 0,
            margin_bottom: 0,
            move_floating: false,
            new_x: req.cur_x,
            new_y: req.cur_y,
        }
    }
}

pub fn compute_osd_pos(avail: Rect, w: i32, h: i32, bottom_margin: i32) -> (i32, i32) {
    let ox = avail.x + (avail.w - w) / 2;
    let oy = avail.y + avail.h - h - bottom_margin;
    (ox, oy)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockCompositor {
        active_output: Option<String>,
        support_info: Option<String>,
        panel: (i32, i32, i32),
    }

    impl CompositorQuery for MockCompositor {
        fn kwin_active_output_name(&self) -> Option<String> {
            self.active_output.clone()
        }

        fn kwin_support_information(&self) -> Option<String> {
            self.support_info.clone()
        }

        fn plasma_panel_info(&self) -> (i32, i32, i32) {
            self.panel
        }
    }

    fn dual_kwin_screens() -> Vec<ScreenInfo> {
        vec![
            ScreenInfo {
                name: "DP-1".into(),
                geo: Rect {
                    x: 0,
                    y: 0,
                    w: 1707,
                    h: 960,
                },
                avail: Rect {
                    x: 0,
                    y: 0,
                    w: 1707,
                    h: 912,
                },
                dpr: 2.0, // wl_output integer ceiling reported by Qt at 1.5x
            },
            ScreenInfo {
                name: "HDMI-A-1".into(),
                geo: Rect {
                    x: 1707,
                    y: 0,
                    w: 1920,
                    h: 1080,
                },
                avail: Rect {
                    x: 1707,
                    y: 0,
                    w: 1920,
                    h: 1036,
                },
                dpr: 1.0,
            },
        ]
    }

    #[test]
    fn parses_kwin_support_information_fractional_scales() {
        let txt = "\
Output 1:
  Name: DP-1
  Enabled: 1
  Geometry: 0,0 1707x960
  Scale: 1.5
Output 2:
  Name: HDMI-A-1
  Scale: 1.25
Output 3:
  Name: BAD-1
  Scale: 99.0
";
        let scales = parse_kwin_output_scales(txt);
        assert_eq!(scales.get("dp-1").copied(), Some(1.5));
        assert_eq!(scales.get("hdmi-a-1").copied(), Some(1.25));
        assert!(!scales.contains_key("bad-1"));
    }

    #[test]
    fn plasma_fractional_sni_point_resolves_onto_active_kwin_monitor() {
        let screens = dual_kwin_screens();
        let mock = MockCompositor {
            active_output: Some("DP-1".into()),
            support_info: Some("Name: DP-1\nScale: 1.5\nName: HDMI-A-1\nScale: 1.0\n".into()),
            panel: (4, 48, 28),
        };
        // Global SNI point delivered by Plasma on DP-1 (local logical x=1600, y=940 -> *1.5 = 2400, 1410)
        let req = TrayPopupReq {
            resolve: TrayResolveReq {
                raw_x: 2400,
                raw_y: 1410,
                has_raw: true,
                cursor_x: 500,
                cursor_y: 500,
                is_xcb: false,
                is_kde_env: true,
            },
            pop_w: 380,
            pop_h: 520,
            edge_pad: 8,
            is_layer_shell: true,
        };
        let scales = query_kwin_output_scales_uncached(&mock);
        let active = find_kwin_active_screen_idx(&screens, &mock);
        let plan = compute_tray_popup_plan_with_scales(
            &screens,
            &req,
            active,
            &scales,
            mock.plasma_panel_info(),
        );
        assert_eq!(plan.screen_idx, 0);
        assert_eq!(plan.qt_x, 1600);
        assert_eq!(plan.qt_y, 940);
        assert!(plan.layer_anchor_bottom);
        assert!(plan.layer_anchor_left);
        // Clamped within DP-1 right edge: 1707 - 380 - 8 = 1319 (since 1600 - 190 = 1410 > 1319)
        assert_eq!(plan.margin_left, 1319);
    }

    #[test]
    fn x11_fallback_clamps_to_work_area_and_anchors_bottom_on_resize() {
        let screens = dual_kwin_screens();
        let mock = MockCompositor {
            active_output: None,
            support_info: None,
            panel: (4, 44, 24),
        };
        let req = TrayPopupReq {
            resolve: TrayResolveReq {
                raw_x: 3600,
                raw_y: 1050,
                has_raw: true,
                cursor_x: 3600,
                cursor_y: 1050,
                is_xcb: true,
                is_kde_env: true,
            },
            pop_w: 380,
            pop_h: 500,
            edge_pad: 8,
            is_layer_shell: false,
        };
        let plan = compute_tray_popup_plan_with_scales(
            &screens,
            &req,
            None,
            &BTreeMap::new(),
            mock.plasma_panel_info(),
        );
        assert_eq!(plan.screen_idx, 1);
        assert_eq!(plan.avail, screens[1].avail);
        // Right limit: 1707 + 1920 - 380 - 8 = 3239
        assert_eq!(plan.popup_x, 3239);
        // Bottom limit: 1036 - 500 - 8 = 528, anchor_bottom = 1028
        assert_eq!(plan.popup_y, 528);
        assert_eq!(plan.anchor_bottom, 1028);

        // Grow popup height from 500 to 620 -> moves upward keeping bottom fixed at 1028
        let resize = compute_tray_resize_plan(&TrayResizeReq {
            screen_geo: plan.screen_geo,
            avail: plan.avail,
            panel_loc: plan.panel_loc,
            anchor_raw_local_offset: plan.anchor_raw_local_offset,
            anchor_bottom: plan.anchor_bottom,
            cur_x: plan.popup_x,
            cur_y: plan.popup_y,
            pop_w: 380,
            pop_h: 620,
            edge_pad: 8,
            is_layer_shell: false,
        });
        assert!(resize.move_floating);
        assert_eq!(resize.new_y, 408);

        // Extreme height exceeds top margin (avail.y + 8 = 8) -> clamps at top
        let overflow_resize = compute_tray_resize_plan(&TrayResizeReq {
            screen_geo: plan.screen_geo,
            avail: plan.avail,
            panel_loc: plan.panel_loc,
            anchor_raw_local_offset: plan.anchor_raw_local_offset,
            anchor_bottom: plan.anchor_bottom,
            cur_x: plan.popup_x,
            cur_y: resize.new_y,
            pop_w: 380,
            pop_h: 1200,
            edge_pad: 8,
            is_layer_shell: false,
        });
        assert_eq!(overflow_resize.new_y, 8);
    }

    #[test]
    fn left_panel_uses_top_left_anchors_and_updates_margin_on_resize() {
        let screens = vec![ScreenInfo {
            name: "eDP-1".into(),
            geo: Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1200,
            },
            avail: Rect {
                x: 56,
                y: 0,
                w: 1864,
                h: 1200,
            },
            dpr: 1.0,
        }];
        let req = TrayPopupReq {
            resolve: TrayResolveReq {
                raw_x: 24,
                raw_y: 1180,
                has_raw: true,
                cursor_x: 24,
                cursor_y: 1180,
                is_xcb: false,
                is_kde_env: false,
            },
            pop_w: 360,
            pop_h: 500,
            edge_pad: 8,
            is_layer_shell: true,
        };
        let plan = compute_tray_popup_plan_with_scales(
            &screens,
            &req,
            None,
            &BTreeMap::new(),
            (5, 56, 16),
        );
        assert!(plan.layer_anchor_top && plan.layer_anchor_left);
        assert!(!plan.layer_anchor_right && !plan.layer_anchor_bottom);
        // Clamped: 1200 - 500 - 8 = 692
        assert_eq!(plan.margin_top, 692);
        assert_eq!(plan.anchor_raw_local_offset, 1180);

        let resize = compute_tray_resize_plan(&TrayResizeReq {
            screen_geo: plan.screen_geo,
            avail: plan.avail,
            panel_loc: plan.panel_loc,
            anchor_raw_local_offset: plan.anchor_raw_local_offset,
            anchor_bottom: plan.anchor_bottom,
            cur_x: 0,
            cur_y: 0,
            pop_w: 360,
            pop_h: 600,
            edge_pad: 8,
            is_layer_shell: true,
        });
        assert!(resize.update_margins);
        assert_eq!(resize.margin_top, 592);
    }

    #[test]
    fn osd_centers_horizontally_above_bottom_margin() {
        let (ox, oy) = compute_osd_pos(
            Rect {
                x: 1920,
                y: 0,
                w: 2560,
                h: 1400,
            },
            320,
            72,
            62,
        );
        assert_eq!(ox, 1920 + (2560 - 320) / 2);
        assert_eq!(oy, 1400 - 72 - 62);
    }
}

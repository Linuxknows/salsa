#include "salsa_widgets.hpp"

#include <QAbstractItemView>
#include <QColorDialog>
#include <QFont>
#include <QFontMetricsF>
#include <QGridLayout>
#include <QHBoxLayout>
#include <QHeaderView>
#include <QLinearGradient>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPen>
#include <QScrollArea>
#include <QTabWidget>
#include <QTableWidget>
#include <QVBoxLayout>
#include <QWheelEvent>

#include <algorithm>
#include <cmath>

namespace salsa {

namespace {

inline double calcMidPt(double a, double b) { return 0.5 * (a + b); }

QString ampTierLabel(uint32_t tier) {
    switch (tier) {
        case SALSA_HW_AMP_LOW: return QStringLiteral("Amp:Low");
        case SALSA_HW_AMP_MED: return QStringLiteral("Amp:Med");
        case SALSA_HW_AMP_HIGH: return QStringLiteral("Amp:High");
        default: return QStringLiteral("Amp:Auto");
    }
}

QString dynModeShort(uint32_t m) {
    switch (m) {
        case 0: return QStringLiteral("Static EQ");
        case 2: return QStringLiteral("Dyn-Expand");
        default: return QStringLiteral("Dyn-Comp");
    }
}

}  // namespace

// =========================================================================
// SalsaAutoGraphCanvas
// =========================================================================

SalsaAutoGraphCanvas::SalsaAutoGraphCanvas(QWidget* parent) : QWidget(parent) {
    setMinimumSize(620, 400);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Expanding);
    setMouseTracking(true);
    setToolTip(QStringLiteral("Interactive Signal Flow Topology — Displays live app clients, DSP/EQ processing, output sinks, capture mics, and MIDI routes. Click any node to open its Owner's Manual chapter."));
    data_.playApps = {QStringLiteral("(No active playback streams)")};
    data_.captApps = {QStringLiteral("Loopback / Capture Ready")};
}

void SalsaAutoGraphCanvas::setTheme(const SalsaThemeData& theme, int fontPct) {
    theme_ = theme;
    fontPct_ = normalizeFontScalePct(fontPct);
    update();
}

void SalsaAutoGraphCanvas::setSelectedNode(const QString& key) {
    if (selectedNode_ != key) {
        selectedNode_ = key;
        update();
    }
}

void SalsaAutoGraphCanvas::updateData(const AutoGraphSnapshotData& data) {
    data_ = data;
    if (data_.playApps.isEmpty()) {
        data_.playApps = {QStringLiteral("(No active playback streams)")};
    }
    if (data_.captApps.isEmpty()) {
        data_.captApps = {QStringLiteral("Loopback / Capture Ready")};
    }
    update();
}

void SalsaAutoGraphCanvas::advancePulse(double dtSec) {
    if (!isVisible()) return;
    pulsePhase_ = std::fmod(pulsePhase_ + std::max(0.0, dtSec) * 0.55, 1.0);
    update();
}

void SalsaAutoGraphCanvas::mouseMoveEvent(QMouseEvent* event) {
    const QPointF pt = event->position();
    QString hit;
    for (const auto& n : hitNodes_) {
        if (n.rect.contains(pt)) {
            hit = n.key;
            break;
        }
    }
    if (hit != hoveredNode_) {
        hoveredNode_ = hit;
        setCursor(hit.isEmpty() ? Qt::ArrowCursor : Qt::PointingHandCursor);
        if (hit.isEmpty()) {
            setToolTip(QStringLiteral("Interactive Signal Flow Topology — Displays live app clients, DSP/EQ processing, output sinks, capture mics, and MIDI routes. Click any node to open its Owner's Manual chapter."));
        } else {
            QString desc = QStringLiteral("Click node '%1' to open its corresponding Owner's Manual chapter.").arg(hit);
            for (const auto& n : hitNodes_) {
                if (n.key == hit) {
                    if (n.manualKey == QLatin1String("apps") || hit == QLatin1String("play")) {
                        desc = QStringLiteral("Active Playback & App Clients — Native WASAPI, Pulse v35, ALSA, and JACK client streams entering Salsa's lock-free mixer. Click to open manual details.");
                    } else if (n.manualKey == QLatin1String("cap") || hit == QLatin1String("capt")) {
                        desc = QStringLiteral("Capture & Microphone Input — Direct ALSA capture hardware, loopback monitors, RNNoise/HPF/gate Mic APO, and HFP SCO. Click to open manual details.");
                    } else if (n.manualKey == QLatin1String("dsp") || hit == QLatin1String("dsp")) {
                        desc = QStringLiteral("10-Band Dynamic EQ & Spatial DSP — Real-time biquad parametric EQ, dynamic compress/expand, crossfeed, and limiter stage. Click to open manual details.");
                    } else if (n.manualKey == QLatin1String("out") || hit == QLatin1String("sink")) {
                        desc = QStringLiteral("Master Output Endpoint — Direct kernel WaveRT ALSA PCM or BlueZ A2DP L2CAP wireless output stage. Click to open manual details.");
                    } else if (n.manualKey == QLatin1String("midi") || hit == QLatin1String("midi")) {
                        desc = QStringLiteral("DAW MIDI Hub — Lock-free ALSA Sequencer and JACK MIDI event bridge. Click to open manual details.");
                    } else if (n.manualKey == QLatin1String("overview")) {
                        desc = QStringLiteral("Salsa RT Audio Engine — SCHED_FIFO real-time WaveRT mixer coordinating all audio, DSP, Video ScreenCast, and device clocks. Click to open overview.");
                    }
                    break;
                }
            }
            setToolTip(desc);
        }
        update();
    }
}

void SalsaAutoGraphCanvas::mousePressEvent(QMouseEvent* event) {
    if (event->button() == Qt::LeftButton) {
        const QPointF pt = event->position();
        for (const auto& n : hitNodes_) {
            if (n.rect.contains(pt)) {
                selectedNode_ = n.manualKey;
                emit nodeClicked(n.manualKey);
                update();
                return;
            }
        }
        selectedNode_ = QStringLiteral("overview");
        emit nodeClicked(selectedNode_);
        update();
    }
}

void SalsaAutoGraphCanvas::leaveEvent(QEvent*) {
    if (!hoveredNode_.isEmpty()) {
        hoveredNode_.clear();
        setCursor(Qt::ArrowCursor);
        update();
    }
}

void SalsaAutoGraphCanvas::drawWire(
    QPainter& p,
    const QPointF& a,
    const QPointF& b,
    const QColor& col,
    bool dashed,
    bool pulse) {
    QPainterPath path(a);
    const double dx = std::abs(b.x() - a.x()) * 0.55;
    const QPointF c1(a.x() + dx, a.y());
    const QPointF c2(b.x() - dx, b.y());
    path.cubicTo(c1, c2, b);

    QColor glow(col);
    glow.setAlpha(pulse ? 45 : 24);
    p.setBrush(Qt::NoBrush);
    p.setPen(QPen(glow, pulse ? 6.0 : 4.0, Qt::SolidLine, Qt::RoundCap));
    p.drawPath(path);

    QPen wirePen(col, pulse ? 2.2 : 1.7, dashed ? Qt::DashLine : Qt::SolidLine, Qt::RoundCap);
    p.setPen(wirePen);
    p.drawPath(path);

    if (pulse) {
        p.setBrush(theme_.qcolor("wire_pulse", "#ffffff"));
        p.setPen(QPen(col, 1.5));
        for (double off : {0.0, 0.5}) {
            const double t = std::fmod(pulsePhase_ + off, 1.0);
            p.drawEllipse(path.pointAtPercent(t), 3.8, 3.8);
        }
    }
}

void SalsaAutoGraphCanvas::drawNode(
    QPainter& p,
    const QString& key,
    const QString& manualKey,
    const QRectF& r,
    const QString& title,
    const QString& badge,
    const QStringList& lines,
    const QColor& accent,
    bool hasIn,
    bool hasOut) {
    hitNodes_.push_back({key, manualKey, r});
    const bool isSel = (!selectedNode_.isEmpty() && selectedNode_ == manualKey);
    const bool isHov = (hoveredNode_ == key);

    QLinearGradient grad(r.topLeft(), r.bottomLeft());
    grad.setColorAt(0.0, theme_.qcolor((isSel || isHov) ? "node_bg_hover" : "node_bg_top", "#1b2028"));
    grad.setColorAt(1.0, theme_.qcolor("node_bg_bot", "#13161b"));
    p.setBrush(grad);
    const double borderW = isSel ? 2.6 : (isHov ? 2.0 : 1.3);
    p.setPen(QPen((isSel || isHov) ? accent : theme_.qcolor("node_border", "#2c3340"), borderW));
    p.drawRoundedRect(r, 9.0, 9.0);

    // v6.9-beta translucent tinted node header bar
    const double hdrH = std::clamp(r.height() * 0.25, 19.0, 24.0);
    const QRectF hdrRect(r.x() + 1.0, r.y() + 1.0, r.width() - 2.0, hdrH);
    QColor hdrCol(accent);
    hdrCol.setAlpha(isSel ? 85 : 45);
    p.setBrush(hdrCol);
    p.setPen(Qt::NoPen);
    p.drawRoundedRect(hdrRect, 8.0, 8.0);

    QFont titleF(QStringLiteral("Inter"));
    titleF.setPixelSize(std::clamp(scaledPx(11, fontPct_), 10, 15));
    titleF.setWeight(QFont::Bold);

    QFont badgeF(QStringLiteral("Monospace"));
    badgeF.setPixelSize(std::clamp(scaledPx(10, fontPct_), 9, 13));
    badgeF.setWeight(QFont::Bold);
    const double badgeW = std::clamp(QFontMetricsF(badgeF).horizontalAdvance(badge) + 8.0, 48.0, r.width() * 0.42);

    p.setFont(titleF);
    p.setPen(theme_.qcolor("text_bright", "#f8fafc"));
    const QRectF titleRect(r.x() + 10.0, r.y() + 1.5, r.width() - badgeW - 18.0, hdrH);
    p.drawText(titleRect, Qt::AlignLeft | Qt::AlignVCenter,
               QFontMetricsF(titleF).elidedText(title, Qt::ElideRight, titleRect.width()));

    p.setFont(badgeF);
    p.setPen(accent);
    p.drawText(QRectF(r.right() - badgeW - 8.0, r.y() + 1.5, badgeW, hdrH), Qt::AlignRight | Qt::AlignVCenter, badge);

    QFont bodyF(QStringLiteral("Monospace"));
    bodyF.setPixelSize(std::clamp(scaledPx(10, fontPct_), 9, 13));
    p.setFont(bodyF);
    const QFontMetricsF fmBody(bodyF);
    p.setPen(theme_.qcolor("text_secondary", "#cbd5e1"));
    const double bodyTop = r.y() + hdrH + 3.0;
    const double bodyH = std::max(14.0, r.bottom() - bodyTop - 3.0);
    const int maxLines = std::min<int>({4, static_cast<int>(lines.size()), std::max(1, static_cast<int>(bodyH / 13.5))});
    const double stepY = std::min(15.5, bodyH / std::max(1, maxLines));

    for (int i = 0; i < maxLines; ++i) {
        const QRectF lr(r.x() + 10.0, bodyTop + i * stepY, r.width() - 20.0, stepY);
        p.drawText(lr, Qt::AlignLeft | Qt::AlignVCenter,
                   fmBody.elidedText(lines[i], Qt::ElideRight, lr.width()));
    }

    // Input/Output Ports
    p.setBrush(accent);
    p.setPen(QPen(theme_.qcolor("handle_border", "#ffffff"), 1.3));
    if (hasIn) p.drawEllipse(QPointF(r.left(), r.center().y()), 5.0, 5.0);
    if (hasOut) p.drawEllipse(QPointF(r.right(), r.center().y()), 5.0, 5.0);
}

void SalsaAutoGraphCanvas::paintEvent(QPaintEvent*) {
    hitNodes_.clear();
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);

    const double w = static_cast<double>(width());
    const double h = static_cast<double>(height());

    const QColor cSec = theme_.qcolor("accent_secondary", "#38bdf8");
    const QColor cMic = theme_.qcolor("accent_mic", "#f472b6");
    const QColor cPri = theme_.qcolor("accent_primary", "#00e676");
    const QColor cHot = theme_.qcolor("accent_hot", "#fb7185");
    const QColor cWarn = theme_.qcolor("accent_warn", "#f59e0b");
    const QColor cMidi = theme_.qcolor("accent_midi", "#a78bfa");
    const QColor cOut = theme_.qcolor("accent_out", "#34d399");
    const QColor cVid = theme_.qcolor(
        "accent_video", theme_.isLight ? "#0891b2" : "#22d3ee");
    QColor cVidDim(cVid);
    cVidDim.setAlpha(145);

    p.setBrush(theme_.qcolor("bg_canvas", "#0e1116"));
    p.setPen(QPen(theme_.qcolor("border_card", "#262b33"), 1.5));
    p.drawRoundedRect(QRectF(1.0, 1.0, w - 2.0, h - 2.0), 10.0, 10.0);

    p.setPen(QPen(theme_.qcolor("canvas_grid", "#18202c"), 1.0));
    constexpr int step = 28;
    for (int gx = step; gx < static_cast<int>(w); gx += step) p.drawLine(gx, 2, gx, static_cast<int>(h) - 2);
    for (int gy = step; gy < static_cast<int>(h); gy += step) p.drawLine(2, gy, static_cast<int>(w) - 2, gy);

    const double colW = std::min(252.0, (w - 80.0) / 4.2);
    const double gap = (w - 32.0 - colW * 4.0) / 3.0;
    const double x1 = 16.0;
    const double x2 = x1 + colW + gap;
    const double x3 = x2 + colW + gap;
    const double x4 = x3 + colW + gap;

    const double topY = 14.0;
    const double botY = h - 10.0;
    const double divGap = 24.0;
    const double usableH = std::max(390.0, botY - topY);
    const double audioH = std::max(274.0, (usableH - divGap) * 0.72);
    const double vidTop = topY + audioH + divGap;
    const double vidH = std::max(92.0, botY - vidTop);

    // Column 1 (Audio + MIDI Sources)
    const double vg = 8.0;
    const double srcAvail = std::max(240.0, audioH - 2.0 * vg);
    const double hApps = srcAvail * (96.0 / 264.0);
    const double hMic = srcAvail * (84.0 / 264.0);
    const double hMidi = srcAvail * (84.0 / 264.0);
    const QRectF rApps(x1, topY, colW, hApps);
    const QRectF rMic(x1, topY + hApps + vg, colW, hMic);
    const QRectF rMidi(x1, topY + hApps + hMic + 2.0 * vg, colW, hMidi);

    // Column 2 (WaveRT SHM & Audio Engine)
    const QRectF rShm(x2, topY + audioH * 0.04, colW, audioH * 0.42);
    const QRectF rEng(x2, topY + audioH * 0.52, colW, audioH * 0.44);

    // Column 3 (Dynamic EQ & MIDI Hub)
    const QRectF rEq(x3, topY + audioH * 0.06, colW, audioH * 0.46);
    const QRectF rMidiHub(x3, topY + audioH * 0.58, colW, audioH * 0.36);

    // Column 4 (Hardware Sink & Capture/Loopback)
    const QRectF rOut(x4, topY + audioH * 0.04, colW, audioH * 0.43);
    const QRectF rCap(x4, topY + audioH * 0.53, colW, audioH * 0.42);

    // Video ScreenCast Lane (Bottom row across Columns 1..4)
    const QRectF rVidSrc(x1, vidTop, colW, vidH);
    const QRectF rVidBrk(x2, vidTop, colW, vidH);
    const QRectF rVidRel(x3, vidTop, colW, vidH);
    const QRectF rVidCon(x4, vidTop, colW, vidH);

    // Wires (Audio & MIDI)
    drawWire(p, QPointF(rApps.right(), rApps.center().y()), QPointF(rShm.left(), rShm.center().y() - 12.0), cSec);
    drawWire(p, QPointF(rMic.right(), rMic.center().y()), QPointF(rShm.left(), rShm.center().y() + 14.0), cMic);
    drawWire(p, QPointF(rShm.right(), rShm.center().y()), QPointF(rEng.left(), rEng.center().y()), cPri);
    drawWire(p, QPointF(rEng.right(), rEng.center().y() - 10.0), QPointF(rEq.left(), rEq.center().y()), cPri);
    drawWire(p, QPointF(rEq.right(), rEq.center().y()), QPointF(rOut.left(), rOut.center().y()), cHot);
    if (data_.micApoEnabled && data_.micSidetoneEnabled) {
        drawWire(p, QPointF(rEng.right(), rEng.center().y() + 8.0), QPointF(rOut.left(), rOut.center().y() + 18.0), cMic, true, true);
    }
    drawWire(p, QPointF(rEq.right(), rEq.center().y() + 18.0), QPointF(rCap.left(), rCap.center().y() - 12.0), cWarn, true);
    drawWire(p, QPointF(rEng.right(), rEng.center().y() + 16.0), QPointF(rCap.left(), rCap.center().y() + 12.0), cMic);
    drawWire(p, QPointF(rMidi.right(), rMidi.center().y()), QPointF(rMidiHub.left(), rMidiHub.center().y()), cMidi);
    drawWire(p, QPointF(rMidiHub.right(), rMidiHub.center().y()), QPointF(rCap.left(), rCap.center().y() + 26.0), cMidi, true);

    // Divider Banner
    const double divY = topY + audioH + divGap * 0.5;
    QColor divPenCol(cVid);
    divPenCol.setAlpha(95);
    p.setPen(QPen(divPenCol, 1.2, Qt::DashLine));
    p.drawLine(QPointF(16.0, divY), QPointF(w - 16.0, divY));

    const QString sockDisp = data_.pwSocketPath.isEmpty() ? QStringLiteral("pipewire-0") : data_.pwSocketPath;
    const QString divLabel = QStringLiteral(" ZERO-PIPEWIRE WAYLAND VIDEO & SCREENCAST LANE (%1) ").arg(sockDisp);
    const double pillW = std::min(580.0, std::max(280.0, w - 40.0));
    const QRectF pillRect((w - pillW) * 0.5, divY - 9.0, pillW, 18.0);
    p.setBrush(theme_.qcolor("bg_canvas", "#0e1116"));
    p.setPen(QPen(divPenCol, 1.0));
    p.drawRoundedRect(pillRect, 8.0, 8.0);
    QFont divF(QStringLiteral("Monospace"));
    divF.setPixelSize(std::clamp(scaledPx(10, fontPct_), 9, 13));
    divF.setWeight(QFont::Bold);
    p.setFont(divF);
    p.setPen(cVid);
    p.drawText(pillRect, Qt::AlignCenter, divLabel);

    // Video State
    const bool vidLive = data_.pwActiveCasting || data_.pwDmabufStreams > 0 || data_.pwMemfdStreams > 0;
    const QColor vwCol = vidLive ? cVid : cVidDim;
    drawWire(p, QPointF(rVidSrc.right(), rVidSrc.center().y()), QPointF(rVidBrk.left(), rVidBrk.center().y()), vwCol, !vidLive, vidLive);
    drawWire(p, QPointF(rVidBrk.right(), rVidBrk.center().y()), QPointF(rVidRel.left(), rVidRel.center().y()), vwCol, !vidLive, vidLive);
    drawWire(p, QPointF(rVidRel.right(), rVidRel.center().y()), QPointF(rVidCon.left(), rVidCon.center().y()), vwCol, !vidLive, vidLive);
    const bool avSyncActive = vidLive && !data_.captApps.isEmpty() && !data_.captApps.first().contains(QStringLiteral("Ready"));
    drawWire(p, QPointF(rCap.center().x(), rCap.bottom()), QPointF(rVidCon.center().x(), rVidCon.top()), avSyncActive ? cWarn : cVidDim, true, avSyncActive);

    // Draw Audio & MIDI Nodes
    const int livePlayCount = (!data_.playApps.isEmpty() && data_.playApps.first().startsWith('(')) ? 0 : static_cast<int>(data_.playApps.size());
    QStringList appsLines;
    for (int i = 0; i < std::min<int>(3, data_.playApps.size()); ++i) {
        appsLines.append(QStringLiteral("• %1").arg(data_.playApps[i]));
    }
    appsLines.append(QStringLiteral("ABI: WASAPI / ALSA / PA v35 / JACK"));
    drawNode(p, QStringLiteral("apps_in"), QStringLiteral("apps_in"), rApps,
             QStringLiteral("Audio Applications"),
             QStringLiteral("%1 LIVE").arg(livePlayCount),
             appsLines, cSec, false, true);

    const uint32_t capKhz = std::max<uint32_t>(1, data_.capSampleRate / 1000);
    const QString micBadge = QStringLiteral("%1k RAW").arg(capKhz);
    const QString micRateLine = QStringLiteral("Capture: %1 kHz Direct Raw")
                                    .arg(data_.capSampleRate / 1000.0, 0, 'g', 4);
    const QString micBtLine = QStringLiteral("Gain: %1% | HFP SCO: %2")
                                  .arg(data_.micVol)
                                  .arg(data_.btActive ? QStringLiteral("READY") : QStringLiteral("OFF"));
    drawNode(p, QStringLiteral("mic_in"), QStringLiteral("mic_in"), rMic,
             QStringLiteral("Hardware Mic Input"), micBadge,
             {QStringLiteral("Dev: %1").arg(data_.hwMic), micRateLine, micBtLine},
             cMic, false, true);

    drawNode(p, QStringLiteral("midi_in"), QStringLiteral("midi_in"), rMidi,
             QStringLiteral("Hardware MIDI In"),
             data_.midiCount > 0 ? QStringLiteral("ACTIVE") : QStringLiteral("RAW+SEQ"),
             {QStringLiteral("Dev: %1").arg(data_.midiCtrl),
              QStringLiteral("Auto: /dev/snd/midiC*D*"),
              QStringLiteral("Key Presses: %1").arg(data_.midiCount)},
             cMidi, false, true);

    const QString shmBadge = data_.hasSnap ? QStringLiteral("%1 µs").arg(data_.rtAvgUs, 0, 'f', 2) : QStringLiteral("0-COPY");
    drawNode(p, QStringLiteral("shm_ring"), QStringLiteral("shm_ring"), rShm,
             QStringLiteral("WaveRT Shared Mem"), shmBadge,
             {QStringLiteral("memfd_create + SCM_RIGHTS"),
              QStringLiteral("Kernel eventfd Wakeups"),
              QStringLiteral("32,768 Stereo f32 Ring"),
              QStringLiteral("100% Zero-Copy Commit")},
             cPri, true, true);

    const double latMs = data_.rate > 0 ? (static_cast<double>(data_.period) * 1000.0 / data_.rate) : 5.33;
    drawNode(p, QStringLiteral("engine_core"), QStringLiteral("engine_core"), rEng,
             QStringLiteral("lwasapi-audiodg"), QStringLiteral("FIXED MASTER"),
             {QStringLiteral("Master: %1 kHz / %2-bit").arg(data_.rate / 1000.0, 0, 'f', 1).arg(data_.bits),
              QStringLiteral("Quantum: %1f (%2 ms)").arg(data_.period).arg(latMs, 0, 'f', 2),
              QStringLiteral("SCHED_FIFO (mlock)"),
              QStringLiteral("PA v35 Clients: %1").arg(data_.paClients)},
             cPri, true, true);

    drawNode(p, QStringLiteral("dyn_eq"), QStringLiteral("dyn_eq"), rEq,
             QStringLiteral("Parametric & Dyn EQ"),
             data_.eqEnabled ? QStringLiteral("ACTIVE") : QStringLiteral("BYPASS"),
             {QStringLiteral("Preset: %1").arg(data_.eqPreset),
              QStringLiteral("Mode: %1 | Lim: %2").arg(dynModeShort(data_.eqMode), data_.limiterEnabled ? QStringLiteral("ON") : QStringLiteral("OFF")),
              QStringLiteral("Crossfeed: %1").arg(data_.crossfeedEnabled ? QStringLiteral("ON (Bauer)") : QStringLiteral("OFF")),
              QStringLiteral("10-Band PK/LS/HS + Limiter")},
             cHot, true, true);

    drawNode(p, QStringLiteral("midi_hub"), QStringLiteral("midi_in"), rMidiHub,
             QStringLiteral("DAW MIDI Hub"), QStringLiteral("LOCK-FREE"),
             {QStringLiteral("Seq Port: LWASAPI-MIDI-Hub"),
              QStringLiteral("Ring: 512 Sample-Accurate Ev"),
              QStringLiteral("Clients: JACK + ALSA Seq"),
              QStringLiteral("Routed Total: %1").arg(data_.midiCount)},
             cMidi, true, true);

    QString hwStageLine = QStringLiteral("Round-Trip: Down to 0.33 ms");
    if (!data_.hwArch.isEmpty() && !data_.hwArch.startsWith(QStringLiteral("Standard"))) {
        hwStageLine = QStringLiteral("%1 | %2%3%4")
                          .arg(data_.hwArch.left(18),
                               ampTierLabel(data_.hwAmpTier),
                               data_.hwDacUnity ? QStringLiteral(" | Unity") : QString(),
                               data_.hwHidConnected ? QStringLiteral(" | HID") : QString());
    }
    const QString btLine = data_.btActive
                               ? QStringLiteral("BT: %1 (%2 ms)").arg(data_.btCodec, QString::number(data_.btDelayMs))
                               : QStringLiteral("Wired Low-Latency Endpoint");
    drawNode(p, QStringLiteral("hw_out"), QStringLiteral("hw_out"), rOut,
             QStringLiteral("Master Output Sink"),
             data_.spkMuted ? QStringLiteral("MUTED") : QStringLiteral("Vol %1%").arg(data_.spkVol),
             {QStringLiteral("Sink: %1").arg(data_.hwSink),
              QStringLiteral("Stage: %1-Bit Studio Master").arg(data_.bits),
              hwStageLine,
              btLine},
             cOut, true, false);

    const int liveCapCount = (!data_.captApps.isEmpty() && data_.captApps.first().contains(QStringLiteral("Ready"))) ? 0 : static_cast<int>(data_.captApps.size());
    QStringList capLines;
    for (int i = 0; i < std::min<int>(2, data_.captApps.size()); ++i) {
        capLines.append(QStringLiteral("• %1").arg(data_.captApps[i]));
    }
    capLines.append(QStringLiteral("Monitor: Post-EQ Loopback"));
    capLines.append(QStringLiteral("Mic Feed: %1k Full-Duplex").arg(capKhz));
    drawNode(p, QStringLiteral("capt_out"), QStringLiteral("hw_out"), rCap,
             QStringLiteral("Audio Capture & Loop"),
             QStringLiteral("%1 REC").arg(liveCapCount),
             capLines, cWarn, true, false);

    // Video ScreenCast Nodes
    drawNode(p, QStringLiteral("vid_src"), QStringLiteral("vid_broker"), rVidSrc,
             QStringLiteral("Video Producer"),
             vidLive ? QStringLiteral("CASTING") : QStringLiteral("STANDBY"),
             {QStringLiteral("Wayland Portal / KWin"),
              QStringLiteral("Streams: %1 DMA / %2 MemFd").arg(data_.pwDmabufStreams).arg(data_.pwMemfdStreams),
              QStringLiteral("Portal: OpenPipeWireRemote")},
             cVid, false, true);

    drawNode(p, QStringLiteral("vid_broker"), QStringLiteral("vid_broker"), rVidBrk,
             QStringLiteral("Video PW Broker"),
             data_.pwBrokerRunning ? QStringLiteral("ONLINE") : QStringLiteral("OFFLINE"),
             {QStringLiteral("Sock: pipewire-0"),
              QStringLiteral("Clients: %1").arg(data_.pwClients.size()),
              QStringLiteral("Audio Rejects: %1").arg(data_.pwAudioRejects)},
             data_.pwBrokerRunning ? cVid : cWarn, true, true);

    const QString capTxt = data_.pwCastLimitFps > 0
                               ? QStringLiteral("%1x%2@%3").arg(data_.pwCastLimitW).arg(data_.pwCastLimitH).arg(data_.pwCastLimitFps)
                               : QStringLiteral("Source (Uncapped)");
    drawNode(p, QStringLiteral("vid_relay"), QStringLiteral("vid_broker"), rVidRel,
             QStringLiteral("Video Relay & Cap"),
             vidLive ? QStringLiteral("%1 FPS").arg(data_.pwActualFps, 0, 'f', 1) : QStringLiteral("IDLE"),
             {QStringLiteral("Cap: %1").arg(capTxt),
              QStringLiteral("Relayed: %1 (%2 FPS)").arg(data_.pwFramesRelayed).arg(data_.pwActualFps, 0, 'f', 1),
              QStringLiteral("Dropped: %1 | SHM Zero-Copy").arg(data_.pwFramesDropped)},
             vidLive ? cPri : cVid, true, true);

    QStringList consLines;
    if (vidLive) {
        consLines = {QStringLiteral("Active Video Consumer"),
                     QStringLiteral("Handoff: OpenPipeWireRemote OK"),
                     QStringLiteral("Audio: Native Salsa PA/WASAPI")};
    } else {
        consLines = {QStringLiteral("Discord / OBS / WebRTC Ready"),
                     QStringLiteral("OpenPipeWireRemote OK"),
                     QStringLiteral("Parallel Native Salsa Audio")};
    }
    drawNode(p, QStringLiteral("vid_cons"), QStringLiteral("vid_broker"), rVidCon,
             QStringLiteral("Video Consumers"),
             vidLive ? QStringLiteral("LIVE") : QStringLiteral("STANDBY"),
             consLines, vidLive ? cWarn : cVid, true, false);
}

// =========================================================================
// SalsaSpectrumCanvas
// =========================================================================

SalsaSpectrumCanvas::SalsaSpectrumCanvas(QWidget* parent) : QWidget(parent) {
    setMinimumHeight(165);
    setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Expanding);
    setMouseTracking(true);
    staticGains_.fill(0.0f);
    bandQ_.fill(1.414f);
    for (int i = 0; i < SALSA_EQ_BANDS; ++i) bandFreqHz_[i] = DEFAULT_BAND_FREQS_HZ[i];
    dynDeltas_.fill(0.0f);
    smoothLevels_.fill(-60.0f);
    setToolTip(QStringLiteral("Drag handles up/down to adjust gain (hold Shift for 0.1 dB fine trim), scroll wheel over handle to adjust Q, right-click handle to reset to 0 dB"));
}

void SalsaSpectrumCanvas::setTheme(const SalsaThemeData& theme, int fontPct) {
    theme_ = theme;
    fontPct_ = normalizeFontScalePct(fontPct);
    update();
}

void SalsaSpectrumCanvas::updateTelemetry(
    bool enabled,
    uint32_t dynMode,
    double thresholdDb,
    const std::array<float, SALSA_EQ_BANDS>& gains,
    const std::array<float, SALSA_EQ_BANDS>& bandQ,
    const std::array<float, SALSA_EQ_BANDS>& bandFreqHz,
    const std::array<float, SALSA_EQ_BANDS>& deltas,
    const std::array<float, SALSA_EQ_BANDS>& levels) {
    bool changed = (enabled_ != enabled || dynMode_ != dynMode || std::abs(thresholdDb_ - thresholdDb) > 1e-4
                    || staticGains_ != gains || bandQ_ != bandQ || bandFreqHz_ != bandFreqHz || dynDeltas_ != deltas);
    enabled_ = enabled;
    dynMode_ = dynMode;
    thresholdDb_ = thresholdDb;
    staticGains_ = gains;
    bandQ_ = bandQ;
    bandFreqHz_ = bandFreqHz;
    dynDeltas_ = deltas;

    const auto nowTp = std::chrono::steady_clock::now();
    double dtSec = 0.016;
    if (lastLevelTick_.time_since_epoch().count() != 0) {
        dtSec = std::clamp(
            std::chrono::duration<double>(nowTp - lastLevelTick_).count(),
            0.001, 0.25);
    }
    lastLevelTick_ = nowTp;
    const float aRise = 1.0f - std::exp(-static_cast<float>(dtSec / 0.014));
    const float aFall = 1.0f - std::exp(-static_cast<float>(dtSec / 0.075));

    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
        const float target = std::clamp(levels[i], -60.0f, 6.0f);
        const float prev = smoothLevels_[i];
        const float alpha = (target > prev) ? aRise : aFall;
        float nxt = prev + alpha * (target - prev);
        if (std::abs(nxt - target) < 0.02f) nxt = target;
        if (nxt != prev) changed = true;
        smoothLevels_[i] = nxt;
    }
    if (changed) update();
}

double SalsaSpectrumCanvas::bandX(int idx, double w) const {
    const double pad = 48.0;
    const double usable = std::max(100.0, w - pad * 2.0);
    return pad + (usable * idx) / (SALSA_EQ_BANDS - 1);
}

double SalsaSpectrumCanvas::gainToY(double db, double h) const {
    const double padTop = 18.0;
    const double padBot = 34.0;
    const double usable = std::max(60.0, h - padTop - padBot);
    const double cl = std::clamp(db, -18.0, 18.0);
    return padTop + usable * (1.0 - (cl + 18.0) / 36.0);
}

double SalsaSpectrumCanvas::yToGain(double y, double h) const {
    const double padTop = 18.0;
    const double padBot = 34.0;
    const double usable = std::max(60.0, h - padTop - padBot);
    const double norm = 1.0 - ((y - padTop) / usable);
    return std::clamp(norm * 36.0 - 18.0, -18.0, 18.0);
}

int SalsaSpectrumCanvas::hitBand(const QPointF& pos) const {
    const double w = static_cast<double>(width());
    const double h = static_cast<double>(height());
    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
        const double bx = bandX(i, w);
        const double by = gainToY(staticGains_[i], h);
        if (std::hypot(pos.x() - bx, pos.y() - by) <= 18.0) return i;
    }
    return -1;
}

void SalsaSpectrumCanvas::mousePressEvent(QMouseEvent* event) {
    const int idx = hitBand(event->position());
    if (event->button() == Qt::LeftButton && idx >= 0) {
        draggingBand_ = idx;
        update();
    } else if (event->button() == Qt::RightButton && idx >= 0) {
        staticGains_[idx] = 0.0f;
        emit bandResetRequested(idx);
        update();
    }
}

void SalsaSpectrumCanvas::mouseMoveEvent(QMouseEvent* event) {
    const double h = static_cast<double>(height());
    const QPointF pos = event->position();
    if (draggingBand_ >= 0) {
        const bool fine = (event->modifiers() & Qt::ShiftModifier) != 0;
        const double stepMult = fine ? 10.0 : 2.0;
        const double newDb = std::round(yToGain(pos.y(), h) * stepMult) / stepMult;
        staticGains_[draggingBand_] = static_cast<float>(newDb);
        emit bandGainDragged(draggingBand_, newDb);
        update();
        return;
    }
    const int hov = hitBand(pos);
    if (hov != hoverBand_) {
        hoverBand_ = hov;
        setCursor(hov >= 0 ? Qt::SizeVerCursor : Qt::ArrowCursor);
        update();
    }
}

void SalsaSpectrumCanvas::mouseReleaseEvent(QMouseEvent* event) {
    if (event->button() == Qt::LeftButton && draggingBand_ >= 0) {
        draggingBand_ = -1;
        setCursor(hoverBand_ >= 0 ? Qt::SizeVerCursor : Qt::ArrowCursor);
        update();
    }
}

void SalsaSpectrumCanvas::wheelEvent(QWheelEvent* event) {
    const int idx = hoverBand_ >= 0 ? hoverBand_ : hitBand(event->position());
    if (idx >= 0) {
        int dy = event->angleDelta().y();
        if (dy == 0) dy = event->pixelDelta().y();
        if (dy != 0) {
            double q = bandQ_[idx];
            q += (dy > 0 ? 0.15 : -0.15);
            q = std::clamp(std::round(q * 100.0) / 100.0, 0.20, 10.00);
            bandQ_[idx] = static_cast<float>(q);
            emit bandQChanged(idx, q);
            update();
            event->accept();
            return;
        }
    }
    QWidget::wheelEvent(event);
}

void SalsaSpectrumCanvas::leaveEvent(QEvent*) {
    if (hoverBand_ >= 0) {
        hoverBand_ = -1;
        setCursor(Qt::ArrowCursor);
        update();
    }
}

void SalsaSpectrumCanvas::paintEvent(QPaintEvent*) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const double w = static_cast<double>(width());
    const double h = static_cast<double>(height());

    p.setBrush(theme_.qcolor("bg_canvas", "#0e1116"));
    p.setPen(QPen(theme_.qcolor("border_card", "#262b33"), 1.5));
    p.drawRoundedRect(QRectF(1.0, 1.0, w - 2.0, h - 2.0), 10.0, 10.0);

    QFont specF(QStringLiteral("Monospace"));
    specF.setPixelSize(std::clamp(scaledPx(10, fontPct_), 9, 14));
    p.setFont(specF);
    const double gridLeft = 40.0;
    const double gridRight = std::max(gridLeft + 40.0, w - 3.0);
    QPainterPath innerClip;
    innerClip.addRoundedRect(QRectF(2.0, 2.0, std::max(10.0, w - 4.0), std::max(10.0, h - 4.0)), 9.0, 9.0);

    for (int db : {18, 12, 6, 0, -6, -12, -18}) {
        const double y = gainToY(static_cast<double>(db), h);
        p.save();
        p.setClipPath(innerClip);
        p.setPen(QPen(theme_.qcolor(db == 0 ? "grid_zero" : "spectrum_grid", "#1e293b"),
                      db == 0 ? 1.2 : 1.0, Qt::DashLine));
        p.drawLine(QPointF(gridLeft, y), QPointF(gridRight, y));
        p.restore();
        p.setPen(theme_.qcolor("text_dim", "#64748b"));
        p.drawText(6, static_cast<int>(y + 4), QStringLiteral("%1%2dB").arg(db >= 0 ? QStringLiteral("+") : QString(), QString::number(db)));
    }

    const double barW = std::max(14.0, (w - 120.0) / 18.0);
    const double baseY = h - 26.0;
    QColor cBarTop = theme_.qcolor("accent_hot", "#fb7185");
    cBarTop.setAlpha(175);
    QColor cBarMid = theme_.qcolor("accent_primary", "#00e676");
    cBarMid.setAlpha(135);
    QColor cBarBot = theme_.qcolor("accent_secondary", "#38bdf8");
    cBarBot.setAlpha(65);

    std::array<QPointF, SALSA_EQ_BANDS> effPts{};
    std::array<QPointF, SALSA_EQ_BANDS> statPts{};

    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
        const double bx = bandX(i, w);
        const double lvlNorm = (smoothLevels_[i] + 60.0) / 66.0;
        const double barH = std::max(3.0, lvlNorm * (h - 48.0));
        QLinearGradient barGrad(bx, baseY - barH, bx, baseY);
        barGrad.setColorAt(0.0, cBarTop);
        barGrad.setColorAt(0.5, cBarMid);
        barGrad.setColorAt(1.0, cBarBot);
        p.setBrush(barGrad);
        p.setPen(Qt::NoPen);
        p.drawRoundedRect(QRectF(bx - barW * 0.5, baseY - barH, barW, barH), 4.0, 4.0);

        const float hz = bandFreqHz_[i];
        const QString lbl = hz >= 1000.0f ? QStringLiteral("%1k").arg(hz / 1000.0f, 0, 'g', 3)
                                          : QStringLiteral("%1").arg(static_cast<int>(std::lround(hz)));
        p.setPen(theme_.qcolor("text_muted", "#94a3b8"));
        p.drawText(QRectF(bx - 28.0, h - 22.0, 56.0, 18.0), Qt::AlignCenter, lbl);

        statPts[i] = QPointF(bx, gainToY(staticGains_[i], h));
        effPts[i] = QPointF(bx, gainToY(staticGains_[i] + dynDeltas_[i], h));
    }

    auto makeExtendedCubic = [gridLeft, gridRight](const std::array<QPointF, SALSA_EQ_BANDS>& pts) -> QPainterPath {
        QPainterPath path(QPointF(gridLeft, pts[0].y()));
        const double lmx = calcMidPt(gridLeft, pts[0].x());
        path.cubicTo(QPointF(lmx, pts[0].y()), QPointF(lmx, pts[0].y()), pts[0]);
        for (int i = 0; i < SALSA_EQ_BANDS - 1; ++i) {
            const double mx = calcMidPt(pts[i].x(), pts[i + 1].x());
            path.cubicTo(QPointF(mx, pts[i].y()), QPointF(mx, pts[i + 1].y()), pts[i + 1]);
        }
        const double rmx = calcMidPt(pts.back().x(), gridRight);
        path.cubicTo(QPointF(rmx, pts.back().y()), QPointF(rmx, pts.back().y()), QPointF(gridRight, pts.back().y()));
        return path;
    };

    const QPainterPath dynPath = makeExtendedCubic(effPts);
    QPainterPath fillPath(dynPath);
    const double zeroY = gainToY(0.0, h);
    fillPath.lineTo(gridRight, zeroY);
    fillPath.lineTo(gridLeft, zeroY);
    fillPath.closeSubpath();

    QColor cFillTop = theme_.qcolor("accent_primary", "#00e676");
    cFillTop.setAlpha(70);
    QColor cFillBot = theme_.qcolor("accent_secondary", "#38bdf8");
    cFillBot.setAlpha(20);
    QLinearGradient fGrad(0.0, 0.0, 0.0, h);
    fGrad.setColorAt(0.0, cFillTop);
    fGrad.setColorAt(1.0, cFillBot);
    p.save();
    p.setClipPath(innerClip);
    p.setBrush(fGrad);
    p.setPen(Qt::NoPen);
    p.drawPath(fillPath);

    p.setBrush(Qt::NoBrush);
    p.setPen(QPen(theme_.qcolor("accent_warn", "#f59e0b"), 1.6, Qt::DotLine));
    p.drawPath(dynPath);

    const QPainterPath statPath = makeExtendedCubic(statPts);
    QColor curveCol = enabled_ ? theme_.qcolor("accent_primary", "#00e676")
                               : theme_.qcolor("text_dim", "#64748b");
    p.setPen(QPen(curveCol, 2.4));
    p.drawPath(statPath);
    p.restore();

    // Band handles
    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
        const bool active = (i == hoverBand_ || i == draggingBand_);
        const double r = active ? 7.0 : 5.0;
        p.setBrush(active ? theme_.qcolor("accent_secondary", "#38bdf8") : curveCol);
        p.setPen(QPen(theme_.qcolor("handle_border", "#ffffff"), active ? 2.0 : 1.4));
        p.drawEllipse(statPts[i], r, r);
    }

    // Active drag/hover HUD tag
    const int infoIdx = draggingBand_ >= 0 ? draggingBand_ : hoverBand_;
    if (infoIdx >= 0) {
        const QString tag = QStringLiteral("%1 Hz | %2%3 dB | Q %4")
                                .arg(static_cast<int>(std::lround(bandFreqHz_[infoIdx])))
                                .arg(staticGains_[infoIdx] >= 0 ? QStringLiteral("+") : QString())
                                .arg(staticGains_[infoIdx], 0, 'f', 1)
                                .arg(bandQ_[infoIdx], 0, 'f', 2);
        QFont hudF(QStringLiteral("Monospace"));
        hudF.setPixelSize(std::clamp(scaledPx(11, fontPct_), 10, 15));
        hudF.setWeight(QFont::Bold);
        p.setFont(hudF);
        const QFontMetricsF fm(hudF);
        const double tw = fm.horizontalAdvance(tag) + 14.0;
        const QRectF tagRect(w - tw - 14.0, 8.0, tw, 20.0);
        p.setBrush(theme_.qcolor("bg_card", "#181b20"));
        p.setPen(QPen(theme_.qcolor("accent_secondary", "#38bdf8"), 1.0));
        p.drawRoundedRect(tagRect, 5.0, 5.0);
        p.setPen(theme_.qcolor("text_bright", "#f8fafc"));
        p.drawText(tagRect, Qt::AlignCenter, tag);
    }
}

// =========================================================================
// MiniSpectrumCanvas / MiniGraphCanvas / MiniSalsaPreview / CustomThemeDialog
// =========================================================================

MiniSpectrumCanvas::MiniSpectrumCanvas(const SalsaThemeData& theme, QWidget* parent)
    : QWidget(parent), theme_(theme) {
    setMinimumHeight(112);
    setMaximumHeight(130);
}

void MiniSpectrumCanvas::setTheme(const SalsaThemeData& theme) {
    theme_ = theme;
    update();
}

void MiniSpectrumCanvas::paintEvent(QPaintEvent*) {
    static constexpr double kGains[10] = {2.5, 2.0, -1.5, -1.0, 0.5, 1.5, 2.5, 3.0, 3.5, 4.0};
    static constexpr double kDeltas[10] = {0.8, 0.6, 0.1, 0.0, 0.3, 0.5, 0.9, 1.1, 1.2, 1.0};
    static constexpr double kLevels[10] = {-18.0, -14.0, -20.0, -24.0, -21.0, -19.0, -16.0, -17.0, -22.0, -26.0};

    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const double w = static_cast<double>(width());
    const double h = static_cast<double>(height());

    auto bxFn = [w](int i) {
        const double pad = 36.0;
        const double usable = std::max(80.0, w - pad * 2.0);
        return pad + (usable * i) / 9.0;
    };
    auto gyFn = [h](double db) {
        const double padTop = 10.0;
        const double padBot = 18.0;
        const double usable = std::max(30.0, h - padTop - padBot);
        return padTop + usable * (1.0 - (std::clamp(db, -18.0, 18.0) + 18.0) / 36.0);
    };

    p.setBrush(theme_.qcolor("bg_canvas", "#0e1116"));
    p.setPen(QPen(theme_.qcolor("border_card", "#262b33"), 1.2));
    p.drawRoundedRect(QRectF(1.0, 1.0, w - 2.0, h - 2.0), 7.0, 7.0);

    QFont msF(QStringLiteral("Monospace"));
    msF.setPixelSize(9);
    p.setFont(msF);
    for (int db : {12, 0, -12}) {
        const double y = gyFn(db);
        p.setPen(QPen(theme_.qcolor(db == 0 ? "grid_zero" : "spectrum_grid", "#1e293b"),
                      db == 0 ? 1.1 : 0.9, Qt::DashLine));
        p.drawLine(30, static_cast<int>(y), static_cast<int>(w) - 10, static_cast<int>(y));
        p.setPen(theme_.qcolor("text_dim", "#64748b"));
        p.drawText(4, static_cast<int>(y + 3), QStringLiteral("%1%2").arg(db >= 0 ? QStringLiteral("+") : QString(), QString::number(db)));
    }

    const double barW = std::max(8.0, (w - 84.0) / 20.0);
    const double baseY = h - 18.0;
    QColor cTop = theme_.qcolor("accent_hot", "#fb7185"); cTop.setAlpha(175);
    QColor cMid = theme_.qcolor("accent_primary", "#00e676"); cMid.setAlpha(135);
    QColor cBot = theme_.qcolor("accent_secondary", "#38bdf8"); cBot.setAlpha(65);

    std::array<QPointF, 10> statPts{}, effPts{};
    for (int i = 0; i < 10; ++i) {
        const double bx = bxFn(i);
        const double barH = std::max(3.0, ((kLevels[i] + 60.0) / 66.0) * (h - 32.0));
        QLinearGradient grad(bx, baseY - barH, bx, baseY);
        grad.setColorAt(0.0, cTop);
        grad.setColorAt(0.5, cMid);
        grad.setColorAt(1.0, cBot);
        p.setBrush(grad);
        p.setPen(Qt::NoPen);
        p.drawRoundedRect(QRectF(bx - barW * 0.5, baseY - barH, barW, barH), 3.0, 3.0);

        p.setPen(theme_.qcolor("text_muted", "#94a3b8"));
        p.drawText(QRectF(bx - 18.0, h - 16.0, 36.0, 14.0), Qt::AlignCenter, QString::fromLatin1(BAND_FREQS_SHORT[i]));

        statPts[i] = QPointF(bx, gyFn(kGains[i]));
        effPts[i] = QPointF(bx, gyFn(kGains[i] + kDeltas[i]));
    }

    QPainterPath dynPath(effPts[0]), statPath(statPts[0]);
    for (int i = 0; i < 9; ++i) {
        const double mx = calcMidPt(effPts[i].x(), effPts[i + 1].x());
        dynPath.cubicTo(QPointF(mx, effPts[i].y()), QPointF(mx, effPts[i + 1].y()), effPts[i + 1]);
        statPath.cubicTo(QPointF(mx, statPts[i].y()), QPointF(mx, statPts[i + 1].y()), statPts[i + 1]);
    }
    QPainterPath fillPath(dynPath);
    const double zeroY = gyFn(0.0);
    fillPath.lineTo(effPts[9].x(), zeroY);
    fillPath.lineTo(effPts[0].x(), zeroY);
    fillPath.closeSubpath();
    QColor fTop = theme_.qcolor("accent_primary", "#00e676"); fTop.setAlpha(65);
    QColor fBot = theme_.qcolor("accent_secondary", "#38bdf8"); fBot.setAlpha(20);
    QLinearGradient fg(0, 0, 0, h);
    fg.setColorAt(0.0, fTop);
    fg.setColorAt(1.0, fBot);
    p.setBrush(fg);
    p.setPen(Qt::NoPen);
    p.drawPath(fillPath);

    p.setBrush(Qt::NoBrush);
    p.setPen(QPen(theme_.qcolor("accent_warn", "#f59e0b"), 1.5, Qt::DotLine));
    p.drawPath(dynPath);
    p.setPen(QPen(theme_.qcolor("accent_primary", "#00e676"), 2.2));
    p.drawPath(statPath);

    p.setPen(QPen(theme_.qcolor("handle_border", "#ffffff"), 1.2));
    p.setBrush(theme_.qcolor("accent_primary", "#00e676"));
    for (const auto& pt : statPts) p.drawEllipse(pt, 3.6, 3.6);
}

MiniGraphCanvas::MiniGraphCanvas(const SalsaThemeData& theme, QWidget* parent)
    : QWidget(parent), theme_(theme) {
    setMinimumHeight(145);
}

void MiniGraphCanvas::setTheme(const SalsaThemeData& theme) {
    theme_ = theme;
    update();
}

void MiniGraphCanvas::paintEvent(QPaintEvent*) {
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);
    const double w = static_cast<double>(width());
    const double h = static_cast<double>(height());

    p.setBrush(theme_.qcolor("bg_canvas", "#0e1116"));
    p.setPen(QPen(theme_.qcolor("border_card", "#262b33"), 1.2));
    p.drawRoundedRect(QRectF(1.0, 1.0, w - 2.0, h - 2.0), 7.0, 7.0);

    p.setPen(QPen(theme_.qcolor("canvas_grid", "#18202c"), 1.0));
    for (int gx = 18; gx < static_cast<int>(w); gx += 18) p.drawLine(gx, 2, gx, static_cast<int>(h) - 2);
    for (int gy = 18; gy < static_cast<int>(h); gy += 18) p.drawLine(2, gy, static_cast<int>(w) - 2, gy);

    const double colW = (w - 50.0) / 3.0;
    const double x1 = 12.0;
    const double x2 = x1 + colW + 13.0;
    const double x3 = x2 + colW + 13.0;
    const double rowH = (h - 26.0) / 2.0;
    const QRectF n1(x1, 10.0, colW, rowH);
    const QRectF n2(x1, 16.0 + rowH, colW, rowH);
    const QRectF n3(x2, 10.0 + rowH * 0.45, colW, rowH * 1.1);
    const QRectF n4(x3, 10.0, colW, rowH);
    const QRectF n5(x3, 16.0 + rowH, colW, rowH);

    auto wire = [&](QPointF a, QPointF b, QColor col) {
        QPainterPath path(a);
        const double dx = std::abs(b.x() - a.x()) * 0.55;
        path.cubicTo(a.x() + dx, a.y(), b.x() - dx, b.y(), b.x(), b.y());
        p.setPen(QPen(col, 1.8));
        p.drawPath(path);
        p.setBrush(theme_.qcolor("wire_pulse", "#ffffff"));
        p.setPen(Qt::NoPen);
        p.drawEllipse(path.pointAtPercent(0.5), 2.5, 2.5);
    };

    wire(QPointF(n1.right(), n1.center().y()), QPointF(n3.left(), n3.center().y() - 6), theme_.qcolor("accent_secondary", "#38bdf8"));
    wire(QPointF(n2.right(), n2.center().y()), QPointF(n3.left(), n3.center().y() + 6), theme_.qcolor("accent_midi", "#a78bfa"));
    wire(QPointF(n3.right(), n3.center().y()), QPointF(n4.left(), n4.center().y()), theme_.qcolor("accent_primary", "#00e676"));
    wire(QPointF(n3.right(), n3.center().y() + 8), QPointF(n5.left(), n5.center().y()), theme_.qcolor("accent_warn", "#f59e0b"));

    auto box = [&](const QRectF& r, const QString& title, const QString& badge, const QColor& acc, bool hot = false) {
        QLinearGradient lg(r.topLeft(), r.bottomLeft());
        lg.setColorAt(0.0, theme_.qcolor(hot ? "node_bg_hover" : "node_bg_top", "#1b2028"));
        lg.setColorAt(1.0, theme_.qcolor("node_bg_bot", "#13161b"));
        p.setBrush(lg);
        p.setPen(QPen(hot ? acc : theme_.qcolor("node_border", "#2c3340"), 1.2));
        p.drawRoundedRect(r, 6.0, 6.0);
        QFont mgF(QStringLiteral("Inter"));
        mgF.setPixelSize(9);
        mgF.setWeight(QFont::Bold);
        p.setFont(mgF);
        p.setPen(theme_.qcolor("text_bright", "#f8fafc"));
        p.drawText(QRectF(r.left() + 5, r.top() + 3, r.width() - 36, 15), Qt::AlignVCenter | Qt::AlignLeft, title);
        p.setPen(acc);
        p.drawText(QRectF(r.right() - 36, r.top() + 3, 31, 15), Qt::AlignRight | Qt::AlignVCenter, badge);
    };

    box(n1, QStringLiteral("Apps"), QStringLiteral("LIVE"), theme_.qcolor("accent_secondary", "#38bdf8"));
    box(n2, QStringLiteral("Mic + MIDI"), QStringLiteral("48k"), theme_.qcolor("accent_midi", "#a78bfa"));
    box(n3, QStringLiteral("audiodg"), QStringLiteral("FIFO"), theme_.qcolor("accent_primary", "#00e676"), true);
    box(n4, QStringLiteral("Output"), QStringLiteral("100%"), theme_.qcolor("accent_out", "#34d399"));
    box(n5, QStringLiteral("Video"), QStringLiteral("BRK"), theme_.qcolor("accent_warn", "#f59e0b"));
}

MiniSalsaPreview::MiniSalsaPreview(const SalsaThemeData& theme, QWidget* parent)
    : QFrame(parent), theme_(theme) {
    setObjectName(QStringLiteral("MiniSalsaRoot"));
    auto* root = new QVBoxLayout(this);
    root->setContentsMargins(8, 8, 8, 8);
    root->setSpacing(6);

    auto* topCard = new QFrame(this);
    topCard->setObjectName(QStringLiteral("Card"));
    auto* topLay = new QHBoxLayout(topCard);
    topLay->setContentsMargins(8, 6, 8, 6);
    topLay->setSpacing(6);

    auto* tCol = new QVBoxLayout();
    tCol->setSpacing(1);
    auto* lblT = new QLabel(QStringLiteral("SALSA — LINUX WASAPI STUDIO"), topCard);
    lblT->setObjectName(QStringLiteral("TitleBanner"));
    auto* lblSub = new QLabel(QStringLiteral("WaveRT Zero-Copy • 192 kHz / 32-Bit • 10-Band Dynamic EQ"), topCard);
    lblSub->setObjectName(QStringLiteral("SubBanner"));
    tCol->addWidget(lblT);
    tCol->addWidget(lblSub);
    topLay->addLayout(tCol, 2);

    struct MiniHdr { const char* lbl; const char* opt1; const char* opt2; };
    for (const MiniHdr& mh : {
             MiniHdr{"Rate:", "48,000 Hz", "192,000 Hz"},
             MiniHdr{"Quantum:", "256 frames", "512 frames"},
             MiniHdr{"Theme:", "Custom", "Catppuccin Mocha"}}) {
        auto* col = new QVBoxLayout();
        col->setSpacing(1);
        col->addWidget(new QLabel(QString::fromLatin1(mh.lbl), topCard));
        auto* cmb = new QComboBox(topCard);
        cmb->addItems({QString::fromLatin1(mh.opt1), QString::fromLatin1(mh.opt2)});
        cmb->setToolTip(QStringLiteral("Interactive Preview Dropdown — Test how dropdown menus and popup lists look in this custom theme."));
        col->addWidget(cmb);
        topLay->addLayout(col);
    }

    auto* engCol = new QVBoxLayout();
    engCol->setSpacing(2);
    auto* lblLat = new QLabel(QStringLiteral("5.33 ms | SCHED_FIFO"), topCard);
    lblLat->setObjectName(QStringLiteral("LatencyLabel"));
    auto* btnEng = new QPushButton(QStringLiteral("SALSA NATIVE"), topCard);
    btnEng->setObjectName(QStringLiteral("PrimaryBtn"));
    btnEng->setToolTip(QStringLiteral("Interactive Preview Primary Button — Shows primary button styling and hover state in this theme."));
    engCol->addWidget(lblLat);
    engCol->addWidget(btnEng);
    topLay->addLayout(engCol);
    root->addWidget(topCard);

    tabs_ = new QTabWidget(this);
    tabs_->setToolTip(QStringLiteral("Preview Tabs — Switch between Studio Mixer & EQ and Topology Graph previews."));
    root->addWidget(tabs_, 1);

    // Tab 1: Studio EQ + Mixer
    auto* tabEq = new QWidget(tabs_);
    auto* eqOuter = new QVBoxLayout(tabEq);
    eqOuter->setContentsMargins(5, 5, 5, 5);
    eqOuter->setSpacing(5);

    auto* eqCard = new QFrame(tabEq);
    eqCard->setObjectName(QStringLiteral("Card"));
    auto* eqLay = new QVBoxLayout(eqCard);
    eqLay->setContentsMargins(7, 6, 7, 6);
    eqLay->setSpacing(4);

    auto* eqBar = new QHBoxLayout();
    eqBar->setSpacing(4);
    auto* chkEq = new QCheckBox(QStringLiteral("Dynamic EQ APO"), eqCard);
    chkEq->setChecked(true);
    chkEq->setObjectName(QStringLiteral("EqEnableCheck"));
    chkEq->setToolTip(QStringLiteral("Interactive Preview Checkbox — Test checkbox checked/unchecked colors under this palette."));
    eqBar->addWidget(chkEq);
    eqBar->addWidget(new QLabel(QStringLiteral("Preset:"), eqCard));
    auto* chipP = new QLabel(QStringLiteral("Saved: Studio Ref"), eqCard);
    chipP->setObjectName(QStringLiteral("SavedChipLabel"));
    eqBar->addWidget(chipP);
    auto* cmbP = new QComboBox(eqCard);
    cmbP->addItems({QStringLiteral("Audiophile"), QStringLiteral("Flat Reference")});
    cmbP->setToolTip(QStringLiteral("Interactive Preview Preset Combo — Shows preset selector colors in this theme."));
    eqBar->addWidget(cmbP);
    auto* btnFlat = new QPushButton(QStringLiteral("Reset Flat"), eqCard);
    btnFlat->setObjectName(QStringLiteral("CompactBtn"));
    btnFlat->setToolTip(QStringLiteral("Interactive Preview Compact Button — Shows secondary/compact button contrast."));
    eqBar->addWidget(btnFlat);
    eqLay->addLayout(eqBar);

    spectrum_ = new MiniSpectrumCanvas(theme_, eqCard);
    spectrum_->setToolTip(QStringLiteral("Live Theme EQ Curve Preview — Displays grid lines, EQ curve stroke, and fill gradient using the current palette tokens."));
    eqLay->addWidget(spectrum_);

    static constexpr double kMGains[10] = {2.5, 2.0, -1.5, -1.0, 0.5, 1.5, 2.5, 3.0, 3.5, 4.0};
    auto* fGrid = new QGridLayout();
    fGrid->setSpacing(3);
    for (int i = 0; i < 10; ++i) {
        auto* col = new QVBoxLayout();
        col->setSpacing(1);
        auto* lv = new QLabel(QStringLiteral("%1%2").arg(kMGains[i] >= 0 ? QStringLiteral("+") : QString()).arg(kMGains[i], 0, 'f', 1), eqCard);
        lv->setObjectName(QStringLiteral("BandValLabel"));
        lv->setAlignment(Qt::AlignCenter);
        auto* sld = new QSlider(Qt::Vertical, eqCard);
        sld->setRange(-180, 180);
        sld->setValue(static_cast<int>(kMGains[i] * 10));
        sld->setMinimumHeight(40);
        sld->setMaximumHeight(48);
        sld->setToolTip(QStringLiteral("Preview EQ Band Fader — Drag to inspect vertical slider handle & groove contrast."));
        auto* lf = new QLabel(QString::fromLatin1(BAND_FREQS_SHORT[i]), eqCard);
        lf->setObjectName(QStringLiteral("BandFreqLabel"));
        lf->setAlignment(Qt::AlignCenter);
        col->addWidget(lv);
        col->addWidget(sld, 0, Qt::AlignHCenter);
        col->addWidget(lf);
        fGrid->addLayout(col, 0, i);
    }
    eqLay->addLayout(fGrid);
    eqOuter->addWidget(eqCard);

    auto* bot = new QHBoxLayout();
    bot->setSpacing(6);
    auto* hwCard = new QFrame(tabEq);
    hwCard->setObjectName(QStringLiteral("Card"));
    auto* hwCol = new QVBoxLayout(hwCard);
    hwCol->setContentsMargins(7, 6, 7, 6);
    hwCol->setSpacing(4);
    auto* lblHw = new QLabel(QStringLiteral("MASTER MIXER"), hwCard);
    lblHw->setObjectName(QStringLiteral("SectionHeader"));
    hwCol->addWidget(lblHw);
    auto* spkRow = new QHBoxLayout();
    spkRow->addWidget(new QLabel(QStringLiteral("Spk 80%"), hwCard));
    auto* sSpk = new QSlider(Qt::Horizontal, hwCard);
    sSpk->setRange(0, 150);
    sSpk->setValue(80);
    sSpk->setToolTip(QStringLiteral("Preview Master Slider — Drag to test horizontal slider fill and thumb styling."));
    spkRow->addWidget(sSpk, 1);
    auto* btnRev = new QPushButton(QStringLiteral("Mute"), hwCard);
    btnRev->setObjectName(QStringLiteral("AccentBtn"));
    btnRev->setToolTip(QStringLiteral("Preview Accent Button — Demonstrates secondary accent button colors in the active palette."));
    spkRow->addWidget(btnRev);
    hwCol->addLayout(spkRow);
    bot->addWidget(hwCard, 1);

    auto* sessCard = new QFrame(tabEq);
    sessCard->setObjectName(QStringLiteral("Card"));
    auto* sessCol = new QVBoxLayout(sessCard);
    sessCol->setContentsMargins(7, 6, 7, 6);
    auto* lblSh = new QLabel(QStringLiteral("ACTIVE SESSIONS"), sessCard);
    lblSh->setObjectName(QStringLiteral("SectionHeader"));
    sessCol->addWidget(lblSh);
    auto* tbl = new QTableWidget(1, 3, sessCard);
    tbl->setHorizontalHeaderLabels({QStringLiteral("App"), QStringLiteral("Mode"), QStringLiteral("Vol")});
    tbl->horizontalHeader()->setSectionResizeMode(0, QHeaderView::Stretch);
    tbl->verticalHeader()->setVisible(false);
    tbl->setMaximumHeight(56);
    tbl->setItem(0, 0, new QTableWidgetItem(QStringLiteral("Firefox")));
    tbl->setItem(0, 1, new QTableWidgetItem(QStringLiteral("PA v35")));
    tbl->setItem(0, 2, new QTableWidgetItem(QStringLiteral("100%")));
    sessCol->addWidget(tbl);
    bot->addWidget(sessCard, 1);
    eqOuter->addLayout(bot);

    tabs_->addTab(tabEq, QStringLiteral("Studio Mixer && EQ"));

    // Tab 2: Auto-Graph Topology Preview
    auto* tabTopo = new QWidget(tabs_);
    auto* topoLay = new QVBoxLayout(tabTopo);
    topoLay->setContentsMargins(6, 6, 6, 6);
    graph_ = new MiniGraphCanvas(theme_, tabTopo);
    topoLay->addWidget(graph_, 1);
    tabs_->addTab(tabTopo, QStringLiteral("Topology Graph"));

    setTheme(theme_);
}

void MiniSalsaPreview::setTheme(const SalsaThemeData& theme) {
    theme_ = theme;
    setPalette(buildSalsaPalette(theme_));
    setStyleSheet(buildSalsaStylesheet(theme_, true, 100));
    if (spectrum_) spectrum_->setTheme(theme_);
    if (graph_) graph_->setTheme(theme_);
}

ColorSwatchRow::ColorSwatchRow(
    const QString& key,
    const QString& label,
    const QString& hint,
    const QString& initialHex,
    QWidget* parent)
    : QWidget(parent), key_(key), hex_(validHexColor(initialHex, QStringLiteral("#ffffff"))) {
    auto* lay = new QHBoxLayout(this);
    lay->setContentsMargins(2, 2, 2, 2);
    lay->setSpacing(6);

    btnSwatch_ = new QPushButton(this);
    btnSwatch_->setFixedSize(28, 22);
    btnSwatch_->setToolTip(QStringLiteral("%1 — click to open color wheel").arg(hint));
    connect(btnSwatch_, &QPushButton::clicked, this, &ColorSwatchRow::openPicker);
    lay->addWidget(btnSwatch_);

    auto* lblCol = new QVBoxLayout();
    lblCol->setContentsMargins(0, 0, 0, 0);
    lblCol->setSpacing(0);
    auto* lblTitle = new QLabel(label, this);
    lblTitle->setStyleSheet(QStringLiteral("font-weight: 600;"));
    auto* lblHint = new QLabel(hint, this);
    lblHint->setStyleSheet(QStringLiteral("font-size: 10px; opacity: 0.72;"));
    lblCol->addWidget(lblTitle);
    lblCol->addWidget(lblHint);
    lay->addLayout(lblCol, 1);

    txtHex_ = new QLineEdit(hex_, this);
    txtHex_->setFixedWidth(82);
    txtHex_->setMaxLength(9);
    txtHex_->setToolTip(QStringLiteral("Hex color for %1").arg(key));
    connect(txtHex_, &QLineEdit::textEdited, this, &ColorSwatchRow::onHexEdited);
    lay->addWidget(txtHex_);

    updateSwatchVisual();
}

void ColorSwatchRow::setHex(const QString& hexColor) {
    hex_ = validHexColor(hexColor, hex_);
    syncing_ = true;
    if (txtHex_->text().toLower() != hex_) txtHex_->setText(hex_);
    syncing_ = false;
    updateSwatchVisual();
}

void ColorSwatchRow::updateSwatchVisual() {
    const QColor c(hex_);
    const QString border = c.lightness() < 128 ? QStringLiteral("#ffffff") : QStringLiteral("#0f172a");
    btnSwatch_->setStyleSheet(
        QStringLiteral(
            "QPushButton, QPushButton:hover, QPushButton:pressed {"
            " background-color: %1; border: 1px solid %2; border-radius: 4px; padding: 0px;"
            "}")
            .arg(hex_, border));
}

void ColorSwatchRow::onHexEdited(const QString& txt) {
    if (syncing_) return;
    const QString s = txt.trimmed();
    if (s.size() == 7 && s.startsWith('#') && QColor::isValidColorName(s)) {
        hex_ = QColor(s).name(QColor::HexRgb).toLower();
        updateSwatchVisual();
        emit colorChanged(key_, hex_);
    }
}

void ColorSwatchRow::openPicker() {
    const QString origHex = hex_;
    QColorDialog dlg(QColor(origHex), this);
    dlg.setWindowTitle(QStringLiteral("Pick Color — %1").arg(key_));
    dlg.setOption(QColorDialog::DontUseNativeDialog, true);
    connect(&dlg, &QColorDialog::currentColorChanged, this, [this](const QColor& col) {
        if (col.isValid()) {
            const QString clean = col.name(QColor::HexRgb).toLower();
            setHex(clean);
            emit colorChanged(key_, clean);
        }
    });
    if (dlg.exec() == QDialog::Accepted && dlg.selectedColor().isValid()) {
        const QString clean = dlg.selectedColor().name(QColor::HexRgb).toLower();
        setHex(clean);
        emit colorChanged(key_, clean);
    } else {
        setHex(origHex);
        emit colorChanged(key_, origHex);
    }
}

SalsaCustomThemeDialog::SalsaCustomThemeDialog(
    const SalsaThemeData& initialTheme,
    int fontPct,
    QWidget* parent)
    : QDialog(parent), fontPct_(normalizeFontScalePct(fontPct)) {
    setWindowTitle(QStringLiteral("Salsa Studio — Custom Theme Studio (Live Interactive Preview)"));
    resize(scaledPx(1180, fontPct_), scaledPx(720, fontPct_));
    workingTheme_ = normalizeThemeData(initialTheme);
    initialSnapshot_ = workingTheme_;
    presetNames_ = allThemeNames(false);

    previewTimer_ = new QTimer(this);
    previewTimer_->setSingleShot(true);
    previewTimer_->setInterval(16);
    connect(previewTimer_, &QTimer::timeout, this, &SalsaCustomThemeDialog::flushPreviewUpdate);

    buildUi();
    refreshAllControlsFromTheme();
    flushPreviewUpdate();
}

void SalsaCustomThemeDialog::resetForOpen(const SalsaThemeData& initialTheme, int fontPct) {
    fontPct_ = normalizeFontScalePct(fontPct);
    workingTheme_ = normalizeThemeData(initialTheme);
    initialSnapshot_ = workingTheme_;
    if (cmbClone_ && !workingTheme_.name.isEmpty()) {
        if (int idx = cmbClone_->findText(workingTheme_.name); idx >= 0) {
            cmbClone_->setCurrentIndex(idx);
        }
    }
    refreshAllControlsFromTheme();
    flushPreviewUpdate();
}

void SalsaCustomThemeDialog::buildUi() {
    auto* root = new QVBoxLayout(this);
    root->setContentsMargins(12, 10, 12, 10);
    root->setSpacing(10);

    auto* topRow = new QHBoxLayout();
    topRow->setSpacing(8);
    topRow->addWidget(new QLabel(QStringLiteral("Clone Base Palette:"), this));
    cmbClone_ = new QComboBox(this);
    cmbClone_->setToolTip(QStringLiteral("Base Theme Preset — Pick an existing theme to copy into the custom palette editor."));
    for (const QString& n : presetNames_) {
        if (n != QLatin1String(CUSTOM_THEME_NAME)) cmbClone_->addItem(n);
    }
    if (!workingTheme_.name.isEmpty()) {
        if (int idx = cmbClone_->findText(workingTheme_.name); idx >= 0) {
            cmbClone_->setCurrentIndex(idx);
        }
    }
    topRow->addWidget(cmbClone_);

    auto* btnClone = new QPushButton(QStringLiteral("Load Base"), this);
    btnClone->setToolTip(QStringLiteral("Load Base Palette — Overwrites the working tokens with the selected preset's colors so you can customize from it."));
    connect(btnClone, &QPushButton::clicked, this, [this]() {
        const QString name = cmbClone_->currentText();
        if (!name.isEmpty()) {
            workingTheme_ = themeByName(name);
            refreshAllControlsFromTheme();
            flushPreviewUpdate();
        }
    });
    topRow->addWidget(btnClone);

    auto* btnAutoShade = new QPushButton(QStringLiteral("Auto-Shade Supporting Colors"), this);
    btnAutoShade->setToolTip(QStringLiteral("Derives cards, borders, grids, node gradients, and buttons from Window BG + Primary/Secondary/Hot accents"));
    connect(btnAutoShade, &QPushButton::clicked, this, [this]() {
        workingTheme_ = deriveHarmonyThemeData(workingTheme_);
        refreshAllControlsFromTheme();
        flushPreviewUpdate();
    });
    topRow->addWidget(btnAutoShade);

    chkLight_ = new QCheckBox(QStringLiteral("Light Mode Polarity"), this);
    chkLight_->setToolTip(QStringLiteral("Light Polarity — Flips text/surface contrast shading math for bright background palettes."));
    connect(chkLight_, &QCheckBox::toggled, this, [this](bool on) {
        workingTheme_.isLight = on;
        schedulePreviewUpdate();
    });
    topRow->addWidget(chkLight_);

    topRow->addStretch(1);
    chkMirrorMain_ = new QCheckBox(QStringLiteral("Mirror Live on Main Salsa Window"), this);
    chkMirrorMain_->setToolTip(QStringLiteral("Mirror Live — Previews unsaved custom palette edits directly on the main Salsa Studio windows as you edit."));
    connect(chkMirrorMain_, &QCheckBox::toggled, this, [this](bool on) {
        if (on) emit liveThemeChanged(workingTheme_);
        else emit mirrorStopped();
    });
    topRow->addWidget(chkMirrorMain_);
    root->addLayout(topRow);

    auto* body = new QHBoxLayout();
    body->setSpacing(12);

    tokenTabs_ = new QTabWidget(this);
    tokenTabs_->setToolTip(QStringLiteral("Color Token Categories — Switch tabs to inspect and edit Base, Accents, EQ/Graph, and Control color tokens."));
    tokenTabs_->setUsesScrollButtons(false);
    tokenTabs_->setMinimumWidth(420);
    tokenTabs_->setMaximumWidth(490);

    SalsaStr groupsJson(salsa_theme_token_groups_json());
    const QJsonArray groupsArr = parseJsonArr(groupsJson.toQString());
    for (const QJsonValue& gv : groupsArr) {
        const QJsonObject gobj = gv.toObject();
        const QString cat = gobj.value(QStringLiteral("category")).toString();
        const QJsonArray toks = gobj.value(QStringLiteral("tokens")).toArray();

        auto* scroll = new QScrollArea(tokenTabs_);
        scroll->setWidgetResizable(true);
        scroll->setFrameShape(QFrame::NoFrame);
        auto* page = new QWidget(scroll);
        auto* pageLay = new QVBoxLayout(page);
        pageLay->setContentsMargins(6, 6, 6, 6);
        pageLay->setSpacing(4);
        for (const QJsonValue& tv : toks) {
            const QJsonObject tobj = tv.toObject();
            const QString key = tobj.value(QStringLiteral("key")).toString();
            const QString lbl = tobj.value(QStringLiteral("label")).toString();
            const QString desc = tobj.value(QStringLiteral("desc")).toString();
            auto* row = new ColorSwatchRow(key, lbl, desc, workingTheme_.color(key), page);
            connect(row, &ColorSwatchRow::colorChanged, this, [this](const QString& k, const QString& hex) {
                workingTheme_.colors.insert(k, hex);
                schedulePreviewUpdate();
            });
            swatchRows_.insert(key, row);
            pageLay->addWidget(row);
        }
        pageLay->addStretch(1);
        scroll->setWidget(page);
        tokenTabs_->addTab(scroll, cat);
    }
    body->addWidget(tokenTabs_);

    auto* rightCard = new QFrame(this);
    rightCard->setObjectName(QStringLiteral("Card"));
    auto* rightLay = new QVBoxLayout(rightCard);
    rightLay->setContentsMargins(10, 8, 10, 10);
    rightLay->setSpacing(6);

    auto* prevHdr = new QHBoxLayout();
    auto* lblPrev = new QLabel(QStringLiteral("LIVE MINI EXACT COPY OF SALSA GUI (INTERACTIVE)"), rightCard);
    lblPrev->setObjectName(QStringLiteral("SectionHeader"));
    prevHdr->addWidget(lblPrev);
    prevHdr->addStretch(1);
    lblSwatchStrip_ = new QLabel(QStringLiteral("● ● ● ● ●"), rightCard);
    lblSwatchStrip_->setStyleSheet(QStringLiteral("font-size: 13px; letter-spacing: 2px;"));
    prevHdr->addWidget(lblSwatchStrip_);
    rightLay->addLayout(prevHdr);

    miniPreview_ = new MiniSalsaPreview(workingTheme_, rightCard);
    rightLay->addWidget(miniPreview_, 1);
    body->addWidget(rightCard, 1);
    root->addLayout(body, 1);

    auto* actRow = new QHBoxLayout();
    actRow->setSpacing(8);
    auto* btnReset = new QPushButton(QStringLiteral("Reset Changes"), this);
    btnReset->setToolTip(QStringLiteral("Reset Changes — Reverts working colors back to what you had when opening Theme Studio."));
    connect(btnReset, &QPushButton::clicked, this, [this]() {
        workingTheme_ = initialSnapshot_;
        refreshAllControlsFromTheme();
        flushPreviewUpdate();
    });
    actRow->addWidget(btnReset);
    actRow->addStretch(1);

    auto* btnCancel = new QPushButton(QStringLiteral("Cancel"), this);
    btnCancel->setToolTip(QStringLiteral("Cancel — Discards unsaved color changes and closes Theme Studio."));
    connect(btnCancel, &QPushButton::clicked, this, &QDialog::reject);
    actRow->addWidget(btnCancel);

    connect(this, &QDialog::rejected, this, [this]() {
        workingTheme_ = initialSnapshot_;
        refreshAllControlsFromTheme();
        flushPreviewUpdate();
        if (chkMirrorMain_ && chkMirrorMain_->isChecked()) {
            chkMirrorMain_->setChecked(false);
        }
    });

    auto* btnApply = new QPushButton(QStringLiteral("Apply"), this);
    btnApply->setToolTip(QStringLiteral("Apply Custom Theme — Saves and activates your Custom palette immediately while keeping Theme Studio open."));
    connect(btnApply, &QPushButton::clicked, this, [this]() {
        initialSnapshot_ = workingTheme_;
        emit themeApplied(workingTheme_);
    });
    actRow->addWidget(btnApply);

    auto* btnSave = new QPushButton(QStringLiteral("Save && Apply Custom Theme"), this);
    btnSave->setToolTip(QStringLiteral("Save & Close — Persists the Custom palette to gui_settings.json, applies it to all windows, and closes the dialog."));
    btnSave->setObjectName(QStringLiteral("PrimaryBtn"));
    connect(btnSave, &QPushButton::clicked, this, [this]() {
        initialSnapshot_ = workingTheme_;
        emit themeApplied(workingTheme_);
        accept();
    });
    actRow->addWidget(btnSave);
    root->addLayout(actRow);
}

void SalsaCustomThemeDialog::refreshAllControlsFromTheme() {
    chkLight_->blockSignals(true);
    chkLight_->setChecked(workingTheme_.isLight);
    chkLight_->blockSignals(false);
    for (auto it = swatchRows_.constBegin(); it != swatchRows_.constEnd(); ++it) {
        it.value()->setHex(workingTheme_.color(it.key()));
    }
}

void SalsaCustomThemeDialog::schedulePreviewUpdate() {
    if (!previewTimer_->isActive()) previewTimer_->start();
}

void SalsaCustomThemeDialog::flushPreviewUpdate() {
    if (miniPreview_) miniPreview_->setTheme(workingTheme_);
    if (lblSwatchStrip_) {
        lblSwatchStrip_->setText(
            QStringLiteral("<span style='color:%1'>■</span> "
                           "<span style='color:%2'>●</span> "
                           "<span style='color:%3'>●</span> "
                           "<span style='color:%4'>●</span> "
                           "<span style='color:%5'>●</span> "
                           "<span style='color:%6'>●</span>")
                .arg(workingTheme_.color("bg_card"),
                     workingTheme_.color("accent_primary"),
                     workingTheme_.color("accent_secondary"),
                     workingTheme_.color("accent_hot"),
                     workingTheme_.color("accent_warn"),
                     workingTheme_.color("accent_midi")));
    }
    if (chkMirrorMain_ && chkMirrorMain_->isChecked()) {
        emit liveThemeChanged(workingTheme_);
    }
}

}  // namespace salsa

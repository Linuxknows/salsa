// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#include "salsa_common.hpp"

#include <QDialog>
#include <QDir>
#include <QFile>
#include <QFont>
#include <QHBoxLayout>
#include <QImage>
#include <QKeyEvent>
#include <QLabel>
#include <QPainter>
#include <QPainterPath>
#include <QPen>
#include <QProcess>
#include <QProcessEnvironment>
#include <QProxyStyle>
#include <QPushButton>
#include <QScreen>
#include <QStandardPaths>
#include <QTextEdit>
#include <QVBoxLayout>
#include <QWheelEvent>
#include <QWindow>

#include <algorithm>
#include <cmath>

namespace salsa {

bool verifyBridgeAbi() {
    return salsa_abi_sizes_ok(
        sizeof(SalsaEngineStatus),
        sizeof(SalsaDeviceList),
        sizeof(SalsaEqConfig),
        sizeof(SalsaParamEqExt),
        sizeof(SalsaMicApoCfg),
        sizeof(SalsaBtCodecCfg),
        sizeof(SalsaHwAnalogCfg));
}

QJsonObject parseJsonObj(const QString& json) {
    if (json.isEmpty()) return {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    return doc.isObject() ? doc.object() : QJsonObject();
}

QJsonArray parseJsonArr(const QString& json) {
    if (json.isEmpty()) return {};
    const QJsonDocument doc = QJsonDocument::fromJson(json.toUtf8());
    return doc.isArray() ? doc.array() : QJsonArray();
}

QString jsonCompact(const QJsonObject& obj) {
    return QString::fromUtf8(QJsonDocument(obj).toJson(QJsonDocument::Compact));
}

QString jsonCompact(const QJsonArray& arr) {
    return QString::fromUtf8(QJsonDocument(arr).toJson(QJsonDocument::Compact));
}

SalsaThemeData themeFromJsonObj(const QJsonObject& obj) {
    SalsaThemeData out;
    const QJsonValue inner = obj.value(QStringLiteral("theme"));
    const QJsonObject src = inner.isObject() ? inner.toObject() : obj;
    out.name = obj.value(QStringLiteral("name")).toString();
    if (out.name.isEmpty()) {
        out.name = src.value(QStringLiteral("name")).toString();
    }
    out.isLight = src.value(QStringLiteral("is_light")).toBool(false);
    for (auto it = src.constBegin(); it != src.constEnd(); ++it) {
        if (it.key() == QStringLiteral("is_light") || it.key() == QStringLiteral("name")) continue;
        if (it.value().isString()) {
            out.colors.insert(it.key(), it.value().toString());
        }
    }
    return out;
}

QJsonObject themeToJsonObj(const SalsaThemeData& theme) {
    QJsonObject obj;
    if (!theme.name.isEmpty()) obj.insert(QStringLiteral("name"), theme.name);
    obj.insert(QStringLiteral("is_light"), theme.isLight);
    for (auto it = theme.colors.constBegin(); it != theme.colors.constEnd(); ++it) {
        obj.insert(it.key(), it.value());
    }
    return obj;
}

SalsaThemeData loadThemeForApp(bool forTray) {
    SalsaStr s(salsa_load_theme_json(forTray));
    return themeFromJsonObj(parseJsonObj(s.toQString()));
}

SalsaThemeData themeByName(const QString& name) {
    const QByteArray utf8 = name.toUtf8();
    SalsaStr s(salsa_get_theme_by_name_json(utf8.constData()));
    return themeFromJsonObj(parseJsonObj(s.toQString()));
}

SalsaThemeData normalizeThemeData(const SalsaThemeData& theme) {
    const QByteArray js = jsonCompact(themeToJsonObj(theme)).toUtf8();
    SalsaStr s(salsa_normalize_theme_json(js.constData()));
    return themeFromJsonObj(parseJsonObj(s.toQString()));
}

SalsaThemeData deriveHarmonyThemeData(const SalsaThemeData& theme) {
    const QByteArray js = jsonCompact(themeToJsonObj(theme)).toUtf8();
    SalsaStr s(salsa_derive_harmony_theme_json(js.constData()));
    return themeFromJsonObj(parseJsonObj(s.toQString()));
}

QStringList allThemeNames(bool forTray) {
    SalsaStr s(salsa_all_theme_names_json(forTray));
    const QJsonArray arr = parseJsonArr(s.toQString());
    QStringList out;
    out.reserve(arr.size());
    for (const QJsonValue& v : arr) {
        if (v.isString()) out.append(v.toString());
    }
    return out;
}

QJsonObject loadGuiSettings() {
    SalsaStr s(salsa_load_gui_settings_json());
    return parseJsonObj(s.toQString());
}

bool updateGuiSettingsPatch(const QJsonObject& patch) {
    const QByteArray js = jsonCompact(patch).toUtf8();
    return salsa_update_gui_settings_json(js.constData()) == 0;
}

QJsonObject hostProfile() {
    SalsaStr s(salsa_host_profile_json());
    return parseJsonObj(s.toQString());
}

QString installRootPath() {
    SalsaStr s(salsa_install_root());
    return s.toQString();
}

int normalizeVolStepPct(int step) {
    for (int s : SUPPORTED_VOL_STEPS) {
        if (s == step) return s;
    }
    return 5;
}

int normalizeFontScalePct(int pct) {
    return salsa_normalize_font_scale(pct);
}

int scaledPx(int basePx, int fontPct) {
    const int p = normalizeFontScalePct(fontPct);
    return std::max(1, static_cast<int>(std::lround(basePx * (static_cast<double>(p) / 100.0))));
}

QColor mixColors(const QColor& a, const QColor& b, double t) {
    const double cl = std::clamp(t, 0.0, 1.0);
    return QColor(
        static_cast<int>(std::lround(a.red() * (1.0 - cl) + b.red() * cl)),
        static_cast<int>(std::lround(a.green() * (1.0 - cl) + b.green() * cl)),
        static_cast<int>(std::lround(a.blue() * (1.0 - cl) + b.blue() * cl)),
        static_cast<int>(std::lround(a.alpha() * (1.0 - cl) + b.alpha() * cl)));
}

QString validHexColor(const QString& candidate, const QString& fallback) {
    const QColor c(candidate.trimmed());
    if (c.isValid()) return c.name(QColor::HexRgb).toLower();
    const QColor fb(fallback);
    return fb.isValid() ? fb.name(QColor::HexRgb).toLower() : QStringLiteral("#ffffff");
}

static QString ensureCheckIconPng(bool darkInk) {
    const QString tmp = QDir::tempPath();
    const QString path = QStringLiteral("%1/salsa_qt_check_%2.png")
                             .arg(tmp, darkInk ? QStringLiteral("dark") : QStringLiteral("light"));
    if (!QFile::exists(path)) {
        QImage img(16, 16, QImage::Format_ARGB32);
        img.fill(Qt::transparent);
        QPainter p(&img);
        p.setRenderHint(QPainter::Antialiasing, true);
        QPen pen(QColor(darkInk ? "#071014" : "#ffffff"), 2.2);
        pen.setCapStyle(Qt::RoundCap);
        pen.setJoinStyle(Qt::RoundJoin);
        p.setPen(pen);
        QPainterPath pathShape;
        pathShape.moveTo(3.2, 8.4);
        pathShape.lineTo(6.6, 11.8);
        pathShape.lineTo(12.8, 4.4);
        p.drawPath(pathShape);
        p.end();
        img.save(path, "PNG");
    }
    return QString(path).replace('\\', '/');
}

QPalette buildSalsaPalette(const SalsaThemeData& t) {
    QPalette pal;
    pal.setColor(QPalette::Window, t.qcolor("bg_main", "#111316"));
    pal.setColor(QPalette::WindowText, t.qcolor("text_primary", "#e2e8f0"));
    pal.setColor(QPalette::Base, t.qcolor("bg_deep", "#0c0e11"));
    pal.setColor(QPalette::AlternateBase, t.qcolor("bg_card", "#181b20"));
    pal.setColor(QPalette::Text, t.qcolor("text_primary", "#e2e8f0"));
    pal.setColor(QPalette::Button, t.qcolor("bg_control", "#232830"));
    pal.setColor(QPalette::ButtonText, t.qcolor("text_bright", "#f8fafc"));
    pal.setColor(QPalette::Highlight, t.qcolor("combo_popup_sel_bg", "#1e3a2f"));
    pal.setColor(QPalette::HighlightedText, t.qcolor("combo_popup_sel_text", "#34d399"));
    pal.setColor(QPalette::ToolTipBase, t.qcolor("bg_card", "#181b20"));
    pal.setColor(QPalette::ToolTipText, t.qcolor("text_primary", "#e2e8f0"));
    return pal;
}

namespace {

// Make left-clicking anywhere on any QSlider seek directly to the cursor and begin dragging
// instead of Qt/Breeze's default single-page-step jump.
class SalsaSliderProxyStyle final : public QProxyStyle {
public:
    using QProxyStyle::QProxyStyle;

    int styleHint(
        StyleHint hint,
        const QStyleOption* option = nullptr,
        const QWidget* widget = nullptr,
        QStyleHintReturn* returnData = nullptr) const override {
        if (hint == SH_Slider_AbsoluteSetButtons) {
            return static_cast<int>(Qt::LeftButton | Qt::MiddleButton);
        }
        if (hint == SH_Slider_PageSetButtons) {
            return static_cast<int>(Qt::NoButton);
        }
        return QProxyStyle::styleHint(hint, option, widget, returnData);
    }
};

}  // namespace

void applyGlobalFontScale(QApplication* app, int fontPct) {
    if (!app) return;
    if (!dynamic_cast<SalsaSliderProxyStyle*>(app->style())) {
        app->setStyle(new SalsaSliderProxyStyle(app->style()));
    }
    QFont f = app->font();
    f.setFamily(QStringLiteral("Inter"));
    f.setPixelSize(scaledPx(12, fontPct));
    app->setFont(f);
}

QString buildSalsaStylesheet(const SalsaThemeData& t, bool mini, int fontPct) {
    const int effPct = mini ? 100 : normalizeFontScalePct(fontPct);
    auto sp = [effPct](int basePx) -> int { return scaledPx(basePx, effPct); };

    const int baseFs = mini ? 9 : sp(12);
    const int titleFs = mini ? 13 : sp(18);
    const int subFs = mini ? 8 : sp(11);
    const int secFs = mini ? 10 : sp(13);
    const int eqChkFs = mini ? 9 : sp(13);
    const int valFs = mini ? 8 : sp(11);
    const int dynFs = mini ? 8 : sp(10);
    const int tabFs = mini ? 9 : sp(12);

    const QString tabPad = mini ? QStringLiteral("4px 10px")
                                : QStringLiteral("%1px %2px").arg(sp(7)).arg(sp(16));
    const QString ctlPad = mini ? QStringLiteral("2px 6px")
                                : QStringLiteral("%1px %2px").arg(sp(5)).arg(sp(10));
    const QString compactPad = mini ? QStringLiteral("2px 6px")
                                    : QStringLiteral("%1px %2px").arg(sp(3)).arg(sp(6));
    const QString savedPad = mini ? QStringLiteral("2px 6px")
                                  : QStringLiteral("%1px %2px").arg(sp(3)).arg(sp(8));

    const int cardRadius = mini ? 7 : 10;
    const int ctlRadius = mini ? 5 : 6;
    const int sliderHw = mini ? 10 : sp(14);
    const int sliderVh = mini ? 10 : sp(14);
    const int sliderGroove = mini ? 4 : 6;
    const int sliderMargin = mini ? -3 : -5;
    const int chkSz = mini ? 12 : sp(15);
    const int chkSpacing = mini ? 5 : 7;

    const bool useDarkTick = t.qcolor("accent_primary", "#00e676").lightness() > 145;
    const QString chkIconPath = ensureCheckIconPng(useDarkTick);

    auto c = [&t](const char* key, const char* fb) -> QString {
        return t.color(QString::fromLatin1(key), QString::fromLatin1(fb));
    };

    QString qss;
    qss.reserve(6000);
    qss += QStringLiteral(
               "QMainWindow, QWidget, QDialog {"
               " background-color: %1;"
               " color: %2;"
               " font-family: 'Inter', 'Noto Sans', sans-serif;"
               " font-size: %3px;"
               " }"
               " QFrame#MiniSalsaRoot {"
               " background-color: %1;"
               " border: 2px solid %4;"
               " border-radius: 10px;"
               " }"
               " QLabel { background-color: transparent; color: %2; }"
               " QCheckBox { background-color: transparent; color: %2; spacing: %5px; }"
               " QCheckBox::indicator { width: %6px; height: %6px; border: 1.5px solid %4; border-radius: 4px; background-color: %7; }"
               " QCheckBox::indicator:hover { border-color: %8; }"
               " QCheckBox::indicator:checked { background-color: %9; border-color: %9; image: url(%10); }")
               .arg(c("bg_main", "#111316"),
                    c("text_primary", "#e2e8f0"),
                    QString::number(baseFs),
                    c("border_control", "#334155"),
                    QString::number(chkSpacing),
                    QString::number(chkSz),
                    c("bg_deep", "#0c0e11"),
                    c("accent_secondary", "#38bdf8"),
                    c("accent_primary", "#00e676"),
                    chkIconPath);

    qss += QStringLiteral(
               " QFrame#Card { background-color: %1; border: 1px solid %2; border-radius: %3px; }"
               " QLabel#TitleBanner { font-size: %4px; font-weight: 800; color: %5; }"
               " QLabel#SubBanner { font-size: %6px; color: %7; font-weight: 600; }"
               " QLabel#SectionHeader { font-size: %8px; font-weight: 700; color: %9; }"
               " QLabel#LatencyLabel { color: %9; font-weight: 700; }"
               " QCheckBox#EqEnableCheck { font-weight: 800; color: %9; font-size: %10px; }"
               " QLabel#PeaksLabel { font-family: Monospace; font-weight: 700; color: %7; }"
               " QLabel#BandValLabel { font-weight: 700; color: %5; font-size: %11px; }"
               " QLabel#BandDynLabel { color: %12; font-size: %13px; font-family: Monospace; }"
               " QLabel#BandFreqLabel { color: %14; font-weight: 700; font-size: %11px; }"
               " QLabel#EndpointsLabel { color: %15; font-size: %11px; }"
               " QLabel#AutoBadgeLabel { color: %7; font-weight: 700; font-size: %11px; }"
               " QLabel#SavedChipLabel { background-color: %16; color: %9; border: 1px solid %17; border-radius: %18px; padding: %19; font-weight: 700; font-size: %11px; }")
               .arg(c("bg_card", "#181b20"),
                    c("border_card", "#262b33"),
                    QString::number(cardRadius),
                    QString::number(titleFs),
                    c("text_bright", "#f8fafc"),
                    QString::number(subFs),
                    c("accent_secondary", "#38bdf8"),
                    QString::number(secFs),
                    c("accent_primary", "#00e676"),
                    QString::number(eqChkFs),
                    QString::number(valFs),
                    c("accent_warn", "#f59e0b"),
                    QString::number(dynFs),
                    c("text_muted", "#94a3b8"),
                    c("text_secondary", "#cbd5e1"),
                    c("bg_deep", "#0c0e11"),
                    c("border_control", "#334155"),
                    QString::number(ctlRadius),
                    savedPad);

    qss += QStringLiteral(
               " QTabWidget::pane { border: 1px solid %1; border-radius: 8px; background-color: %2; }"
               " QTabBar::tab { background-color: %3; color: %4; border: 1px solid %1; border-bottom: none;"
               " border-top-left-radius: 7px; border-top-right-radius: 7px; padding: %5; margin-right: 4px;"
               " font-weight: 700; font-size: %6px; }"
               " QTabBar::tab:hover { background-color: %7; color: %8; }"
               " QTabBar::tab:selected { background-color: %7; color: %9; border-color: %10; }"
               " QComboBox, QPushButton, QSpinBox, QDoubleSpinBox, QLineEdit {"
               " background-color: %7; color: %8; border: 1px solid %10; border-radius: %11px;"
               " padding: %12; font-weight: 600; }"
               " QLineEdit { background-color: %13; }"
               " QPushButton#CompactBtn { padding: %14; font-size: %15px; }"
               " QPushButton#FontStepBtn { padding: %16; font-size: %15px; font-weight: 700; min-width: %17px; }"
               " QComboBox:hover, QPushButton:hover { background-color: %18; border-color: %19; }"
               " QComboBox QAbstractItemView { background-color: %20; color: %8; border: 1px solid %19;"
               " selection-background-color: %21; selection-color: %22; outline: 0; padding: 4px; }")
               .arg(c("border_card", "#262b33"),
                    c("bg_main", "#111316"),
                    c("bg_card", "#181b20"),
                    c("text_muted", "#94a3b8"),
                    tabPad,
                    QString::number(tabFs),
                    c("bg_control", "#232830"),
                    c("text_bright", "#f8fafc"),
                    c("accent_primary", "#00e676"),
                    c("border_control", "#334155"),
                    QString::number(ctlRadius),
                    ctlPad,
                    c("bg_deep", "#0c0e11"),
                    compactPad,
                    QString::number(valFs),
                    QStringLiteral("%1px %2px").arg(sp(3)).arg(sp(7)),
                    QString::number(sp(34)),
                    c("bg_hover", "#2d343f"),
                    c("accent_secondary", "#38bdf8"),
                    c("combo_popup_bg", "#181b20"),
                    c("combo_popup_sel_bg", "#1e3a2f"),
                    c("combo_popup_sel_text", "#34d399"));

    qss += QStringLiteral(
               " QPushButton#PrimaryBtn { background-color: %1; border-color: %2; color: %3; font-weight: 700; }"
               " QPushButton#PrimaryBtn:hover { background-color: %4; }"
               " QPushButton#AccentBtn { background-color: %5; border-color: %6; color: %7; font-weight: 700; }"
               " QPushButton#AccentBtn:hover { background-color: %8; }")
               .arg(c("btn_primary_bg", "#047857"),
                    c("btn_primary_border", "#00e676"),
                    c("btn_primary_text", "#ffffff"),
                    c("btn_primary_hover", "#059669"),
                    c("btn_accent_bg", "#be123c"),
                    c("btn_accent_border", "#fb7185"),
                    c("btn_accent_text", "#ffffff"),
                    c("btn_accent_hover", "#e11d48"));

    qss += QStringLiteral(
               " QSlider::groove:vertical { background: %1; border: 1px solid %2; width: %3px; border-radius: 3px; }"
               " QSlider::add-page:vertical { background: %4; border-radius: 3px; }"
               " QSlider::handle:vertical { background: %4; border: 2px solid %5; height: %6px; margin: 0 %7px; border-radius: 5px; }"
               " QSlider::groove:horizontal { background: %1; border: 1px solid %2; height: %3px; border-radius: 3px; }"
               " QSlider::sub-page:horizontal { background: %8; border-radius: 3px; }"
               " QSlider::handle:horizontal { background: %8; border: 2px solid %5; width: %9px; margin: %7px 0; border-radius: 5px; }")
               .arg(c("bg_deep", "#0c0e11"),
                    c("border_control", "#334155"),
                    QString::number(sliderGroove),
                    c("accent_primary", "#00e676"),
                    c("handle_border", "#ffffff"),
                    QString::number(sliderVh),
                    QString::number(sliderMargin),
                    c("accent_secondary", "#38bdf8"),
                    QString::number(sliderHw));

    qss += QStringLiteral(
               " QTableWidget, QTextBrowser, QTextEdit, QPlainTextEdit {"
               " background-color: %1; color: %2; border: 1px solid %3; border-radius: 8px;"
               " gridline-color: %4; padding: 4px; }"
               " QTableWidget::item:selected { background-color: %5; color: %6; }"
               " QHeaderView::section, QTableCornerButton::section {"
               " background-color: %7; color: %8; padding: 3px 6px; border: 1px solid %4; font-weight: 700; }"
               " QTextBrowser#ManualBrowser { font-size: %9px; line-height: 1.4; color: %2; }"
               " QScrollBar:vertical { background: %1; width: 8px; border-radius: 4px; }"
               " QScrollBar::handle:vertical { background: %3; border-radius: 4px; min-height: 18px; }"
               " QScrollBar::handle:vertical:hover { background: %10; }"
               " QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical,"
               " QScrollBar::add-line:horizontal, QScrollBar::sub-line:horizontal { height: 0px; width: 0px; }"
               " QScrollBar:horizontal { background: %1; height: 8px; border-radius: 4px; }"
               " QScrollBar::handle:horizontal { background: %3; border-radius: 4px; min-width: 18px; }"
               " QProgressBar { background-color: %1; color: %11; border: 1px solid %3; border-radius: 5px;"
               " text-align: center; font-size: %12px; font-weight: 700; min-height: 16px; }"
               " QProgressBar::chunk { background-color: %13; border-radius: 4px; }")
               .arg(c("bg_deep", "#0c0e11"),
                    c("text_primary", "#e2e8f0"),
                    c("border_control", "#334155"),
                    c("border_card", "#262b33"),
                    c("combo_popup_sel_bg", "#1e3a2f"),
                    c("combo_popup_sel_text", "#34d399"),
                    c("bg_control", "#232830"),
                    c("text_muted", "#94a3b8"),
                    QString::number(baseFs + 1),
                    c("bg_hover", "#2d343f"),
                    c("text_bright", "#f8fafc"),
                    QString::number(dynFs),
                    c("accent_primary", "#00e676"));

    return qss;
}

void spawnDetachedWaylandBinary(const QString& binName, const QStringList& args) {
    const QString root = installRootPath();
    QString exePath = QDir(root).filePath(QStringLiteral("build/%1").arg(binName));
    if (!QFile::exists(exePath)) {
        const QString found = QStandardPaths::findExecutable(binName);
        if (!found.isEmpty()) exePath = found;
    }
    if (exePath.isEmpty() || !QFile::exists(exePath)) return;

    QProcess proc;
    QProcessEnvironment env = QProcessEnvironment::systemEnvironment();
    // lwasapi-tray runs under QT_QPA_PLATFORM=xcb for Plasma SNI positioning;
    // child Wayland apps must clear QT_QPA_PLATFORM so they connect natively to Wayland.
    env.remove(QStringLiteral("QT_QPA_PLATFORM"));
    env.remove(QStringLiteral("LWASAPI_TRAY_REPLACED"));
    proc.setProcessEnvironment(env);
    proc.setWorkingDirectory(root);

    const QString systemdRun = QStandardPaths::findExecutable(QStringLiteral("systemd-run"));
    if (!systemdRun.isEmpty()) {
        QStringList scopeArgs{
            QStringLiteral("--user"),
            QStringLiteral("--scope"),
            QStringLiteral("--quiet"),
            QStringLiteral("--"),
            exePath,
        };
        scopeArgs.append(args);
        proc.setProgram(systemdRun);
        proc.setArguments(scopeArgs);
        if (proc.startDetached()) return;
    }

    proc.setProgram(exePath);
    proc.setArguments(args);
    proc.startDetached();
}

void showReportDialog(
    QWidget* parent,
    const QString& title,
    const QString& bodyText,
    const SalsaThemeData& theme,
    int fontPct,
    int width,
    int height) {
    QDialog dlg(parent);
    dlg.setWindowTitle(title);
    dlg.resize(scaledPx(width, fontPct), scaledPx(height, fontPct));
    auto* lay = new QVBoxLayout(&dlg);
    auto* txt = new QTextEdit(&dlg);
    txt->setReadOnly(true);
    txt->setStyleSheet(
        QStringLiteral("background-color: %1; color: %2; font-family: Monospace; font-size: %3px;")
            .arg(theme.color("bg_deep", "#0c0e11"),
                 theme.color("accent_primary", "#00e676"),
                 QString::number(scaledPx(11, fontPct))));
    txt->setPlainText(bodyText);
    lay->addWidget(txt);
    auto* btnClose = new QPushButton(QStringLiteral("Close Report"), &dlg);
    btnClose->setToolTip(QStringLiteral("Close Report — Dismisses this scrollable diagnostic report dialog."));
    QObject::connect(btnClose, &QPushButton::clicked, &dlg, &QDialog::accept);
    lay->addWidget(btnClose);
    dlg.exec();
}

StepVolumeSlider::StepVolumeSlider(
    std::function<int()> stepProvider,
    Qt::Orientation orient,
    QWidget* parent)
    : QSlider(orient, parent), stepFn_(std::move(stepProvider)) {
    setFocusPolicy(Qt::StrongFocus);
}

void StepVolumeSlider::wheelEvent(QWheelEvent* event) {
    int dy = event->angleDelta().y();
    int dx = event->angleDelta().x();
    int raw = dy != 0 ? dy : dx;
    int notches = 1;
    if (raw != 0) {
        notches = std::max<int>(1, static_cast<int>(std::lround(std::abs(raw) / 120.0)));
    } else {
        const QPoint pd = event->pixelDelta();
        raw = pd.y() != 0 ? pd.y() : pd.x();
        notches = std::max<int>(1, static_cast<int>(std::lround(std::abs(raw) / 24.0)));
    }
    if (raw != 0) {
        const int step = std::max(1, stepFn_ ? stepFn_() : 5) * notches;
        const int nxt = std::clamp(value() + (raw > 0 ? step : -step), minimum(), maximum());
        if (nxt != value()) {
            setValue(nxt);
            emit sliderReleased();
        }
        event->accept();
        return;
    }
    QSlider::wheelEvent(event);
}

void StepVolumeSlider::keyPressEvent(QKeyEvent* event) {
    const int k = event->key();
    const int step = std::max(1, stepFn_ ? stepFn_() : 5);
    int delta = 0;
    if (k == Qt::Key_Right || k == Qt::Key_Up) delta = step;
    else if (k == Qt::Key_Left || k == Qt::Key_Down) delta = -step;
    else if (k == Qt::Key_PageUp) delta = step * 2;
    else if (k == Qt::Key_PageDown) delta = -step * 2;
    if (delta != 0) {
        const int nxt = std::clamp(value() + delta, minimum(), maximum());
        if (nxt != value()) {
            setValue(nxt);
            emit sliderReleased();
        }
        event->accept();
        return;
    }
    QSlider::keyPressEvent(event);
}

FontScaleBar::FontScaleBar(int initialPct, bool compact, QWidget* parent)
    : QWidget(parent), pct_(normalizeFontScalePct(initialPct)), compact_(compact) {
    auto* lay = new QHBoxLayout(this);
    lay->setContentsMargins(0, 0, 0, 0);
    lay->setSpacing(compact ? 3 : 4);

    btnMinus_ = new QPushButton(QStringLiteral("Aa-"), this);
    btnMinus_->setObjectName(QStringLiteral("FontStepBtn"));
    btnMinus_->setToolTip(QStringLiteral("Decrease UI font size (Ctrl+-)"));
    btnMinus_->setCursor(Qt::PointingHandCursor);
    connect(btnMinus_, &QPushButton::clicked, this, [this]() { stepBy(-10); });
    lay->addWidget(btnMinus_);

    btnPct_ = new QPushButton(QStringLiteral("%1%").arg(pct_), this);
    btnPct_->setObjectName(QStringLiteral("FontStepBtn"));
    btnPct_->setToolTip(QStringLiteral("Reset UI font zoom to 100% (Ctrl+0)"));
    btnPct_->setCursor(Qt::PointingHandCursor);
    connect(btnPct_, &QPushButton::clicked, this, [this]() {
        if (pct_ != 100) {
            pct_ = 100;
            btnPct_->setText(QStringLiteral("100%"));
            emit fontScaleChanged(pct_);
        }
    });
    lay->addWidget(btnPct_);

    btnPlus_ = new QPushButton(QStringLiteral("Aa+"), this);
    btnPlus_->setObjectName(QStringLiteral("FontStepBtn"));
    btnPlus_->setToolTip(QStringLiteral("Increase UI font size (Ctrl+=)"));
    btnPlus_->setCursor(Qt::PointingHandCursor);
    connect(btnPlus_, &QPushButton::clicked, this, [this]() { stepBy(10); });
    lay->addWidget(btnPlus_);
}

void FontScaleBar::setFontScalePct(int pct) {
    pct_ = normalizeFontScalePct(pct);
    if (btnPct_) btnPct_->setText(QStringLiteral("%1%").arg(pct_));
}

void FontScaleBar::stepBy(int delta) {
    const int next = normalizeFontScalePct(pct_ + delta);
    if (next == pct_) return;
    pct_ = next;
    if (btnPct_) btnPct_->setText(QStringLiteral("%1%").arg(pct_));
    emit fontScaleChanged(pct_);
}

int salsaDetectedOutputHz(const QWidget* w) {
    QScreen* scr = nullptr;
    if (w) {
        if (QWidget* win = w->window()) {
            scr = win->screen();
        }
        if (!scr) scr = w->screen();
    }
    if (!scr) scr = QGuiApplication::primaryScreen();
    if (!scr) return 60;
    const double rr = scr->refreshRate();
    if (rr >= 24.0 && rr <= 500.0) {
        return static_cast<int>(std::lround(rr));
    }
    return 60;
}

int salsaTelemetryIntervalMs(const QWidget* w, int maxHz) {
    const int hz = std::clamp(salsaDetectedOutputHz(w), 30, std::max(30, maxHz));
    return std::clamp(static_cast<int>(std::floor(1000.0 / hz)), 4, 33);
}

int handleLegalCliArgs(int argc, char** argv, const char* appName) {
    for (int i = 1; i < argc; ++i) {
        const int rc = salsa_handle_legal_cli_flag(appName, argv[i]);
        if (rc >= 0) return rc;
    }
    return -1;
}

void showAboutLicenseDialog(
    QWidget* parent,
    const QString& appName,
    const SalsaThemeData& theme,
    int fontPct) {
    QDialog dlg(parent);
    const SalsaStr ver(salsa_version_str());
    dlg.setWindowTitle(
        QStringLiteral("About %1 — Salsa %2 (LGPL-3.0-only)").arg(appName, ver.toQString()));
    dlg.resize(scaledPx(860, fontPct), scaledPx(580, fontPct));
    dlg.setPalette(buildSalsaPalette(theme));
    dlg.setStyleSheet(buildSalsaStylesheet(theme, false, fontPct));

    auto* lay = new QVBoxLayout(&dlg);
    lay->setContentsMargins(12, 10, 12, 10);
    lay->setSpacing(8);

    auto* hdr = new QLabel(
        QStringLiteral("<b>Salsa (LWASAPI) %1 — %2</b><br>"
                       "Copyright (C) 2026 Linuxknows &nbsp;|&nbsp; "
                       "License: <b>GNU Lesser General Public License v3.0 only (LGPL-3.0-only)</b><br>"
                       "<span style=\"color:%3;\">Distributed WITHOUT ANY WARRANTY; without even the implied warranty of "
                       "MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.</span>")
            .arg(ver.toQString(), appName.toHtmlEscaped(), theme.colors.value(QStringLiteral("text_secondary"), QStringLiteral("#94A3B8"))),
        &dlg);
    hdr->setWordWrap(true);
    lay->addWidget(hdr);

    auto* tabRow = new QHBoxLayout();
    tabRow->setSpacing(6);
    auto* btnSummary = new QPushButton(QStringLiteral("Notice & Summary"), &dlg);
    btnSummary->setToolTip(QStringLiteral("Legal Notice & Summary — Shows Salsa copyright, LGPL-3.0-only licensing, no-warranty disclaimer, relinking terms, and optional dlopen disclosure."));
    auto* btnThird = new QPushButton(QStringLiteral("Third-Party / dlopen"), &dlg);
    btnThird->setToolTip(QStringLiteral("Third-Party Libraries — Lists dynamically linked Qt6/audio frameworks and optional runtime dlopen codecs (including libfdk-aac notice)."));
    auto* btnLgpl = new QPushButton(QStringLiteral("LGPL-3.0 (COPYING.LESSER)"), &dlg);
    btnLgpl->setToolTip(QStringLiteral("GNU LGPL v3.0 Text — Displays the full verbatim GNU Lesser General Public License v3.0 additional permissions text (COPYING.LESSER)."));
    auto* btnGpl = new QPushButton(QStringLiteral("GPL-3.0 (COPYING)"), &dlg);
    btnGpl->setToolTip(QStringLiteral("GNU GPL v3.0 Text — Displays the full verbatim GNU General Public License v3.0 base license text (COPYING)."));
    for (QPushButton* b : {btnSummary, btnThird, btnLgpl, btnGpl}) {
        b->setCheckable(true);
        b->setAutoExclusive(true);
        tabRow->addWidget(b);
    }
    tabRow->addStretch(1);
    lay->addLayout(tabRow);

    auto* viewer = new QTextEdit(&dlg);
    viewer->setReadOnly(true);
    QFont mono(QStringLiteral("monospace"), scaledPx(10, fontPct));
    mono.setStyleHint(QFont::Monospace);
    viewer->setFont(mono);
    lay->addWidget(viewer, 1);

    const QByteArray appUtf8 = appName.toUtf8();
    auto loadSection = [viewer, appUtf8](int sec) {
        const SalsaStr txt(salsa_license_section_text(appUtf8.constData(), sec));
        viewer->setPlainText(txt.toQString());
    };
    QObject::connect(btnSummary, &QPushButton::clicked, &dlg, [loadSection]() { loadSection(0); });
    QObject::connect(btnThird, &QPushButton::clicked, &dlg, [loadSection]() { loadSection(1); });
    QObject::connect(btnLgpl, &QPushButton::clicked, &dlg, [loadSection]() { loadSection(2); });
    QObject::connect(btnGpl, &QPushButton::clicked, &dlg, [loadSection]() { loadSection(3); });
    btnSummary->setChecked(true);
    loadSection(0);

    auto* footRow = new QHBoxLayout();
    footRow->addStretch(1);
    auto* btnClose = new QPushButton(QStringLiteral("Close"), &dlg);
    btnClose->setObjectName(QStringLiteral("PrimaryBtn"));
    btnClose->setToolTip(QStringLiteral("Close License Viewer — Returns to the Salsa interface."));
    QObject::connect(btnClose, &QPushButton::clicked, &dlg, &QDialog::accept);
    footRow->addWidget(btnClose);
    lay->addLayout(footRow);

    dlg.exec();
}

}  // namespace salsa

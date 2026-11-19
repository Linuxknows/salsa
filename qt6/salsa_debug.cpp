// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#include "salsa_common.hpp"
#include "salsa_widgets.hpp"

#include <QApplication>
#include <QCloseEvent>
#include <QComboBox>
#include <QFrame>
#include <QHBoxLayout>
#include <QKeySequence>
#include <QLabel>
#include <QMainWindow>
#include <QPushButton>
#include <QShortcut>
#include <QTimer>
#include <QVBoxLayout>

#include <algorithm>
#include <cstdio>
#include <cstring>

namespace salsa {

class SalsaDebugWindow : public QMainWindow {
    Q_OBJECT
public:
    explicit SalsaDebugWindow(int initialTab = 0, QWidget* parent = nullptr)
        : QMainWindow(parent) {
        setWindowTitle(QStringLiteral("Salsa — Standalone Live Architecture & IPC Debugger"));
        resize(1340, 840);

        guiSettings_ = loadGuiSettings();
        lastMtimeNs_ = salsa_gui_settings_mtime_ns();
        fontPct_ = normalizeFontScalePct(salsa_load_font_scale());
        theme_ = loadThemeForApp(false);
        themeName_ = guiSettings_.value(QStringLiteral("theme")).toString(QStringLiteral("Obsidian Glass"));

        auto* central = new QWidget(this);
        setCentralWidget(central);
        auto* root = new QVBoxLayout(central);
        root->setContentsMargins(10, 8, 10, 8);
        root->setSpacing(6);

        auto* topCard = new QFrame(central);
        topCard->setObjectName(QStringLiteral("Card"));
        auto* topLay = new QHBoxLayout(topCard);
        topLay->setContentsMargins(12, 8, 12, 8);
        topLay->setSpacing(8);

        auto* titleCol = new QVBoxLayout();
        titleCol->setSpacing(1);
        auto* lblTitle = new QLabel(QStringLiteral("SALSA LIVE ARCHITECTURE DEBUGGER"), topCard);
        lblTitle->setObjectName(QStringLiteral("TitleBanner"));
        titleCol->addWidget(lblTitle);
        lblSub_ = new QLabel(QStringLiteral("Zero-Overhead Stream, Routing, Process Audit, Hardware Probes & Wayland ScreenCast"), topCard);
        lblSub_->setObjectName(QStringLiteral("SubBanner"));
        lblSub_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
        lblSub_->setMinimumWidth(0);
        titleCol->addWidget(lblSub_);
        topLay->addLayout(titleCol, 1);

        auto* btnStudio = new QPushButton(QStringLiteral("Open Salsa Studio"), topCard);
        btnStudio->setToolTip(QStringLiteral("Open Salsa Studio — Launches the main Salsa mixer, 10-band EQ, and Advanced Options window."));
        connect(btnStudio, &QPushButton::clicked, this, []() {
            spawnDetachedWaylandBinary(QStringLiteral("salsa-studio"), {QStringLiteral("--studio")});
        });
        topLay->addWidget(btnStudio);

        auto* btnAbout = new QPushButton(QStringLiteral("About / LGPL-3.0"), topCard);
        btnAbout->setToolTip(QStringLiteral("About & LGPL-3.0 License — Displays Salsa copyright, no-warranty disclaimer, relinking/source terms, third-party dlopen disclosures, and full COPYING/COPYING.LESSER texts."));
        connect(btnAbout, &QPushButton::clicked, this, [this]() {
            showAboutLicenseDialog(this, QStringLiteral("Salsa Live Debugger"), theme_, fontPct_);
        });
        topLay->addWidget(btnAbout);

        topLay->addWidget(new QLabel(QStringLiteral("Theme:"), topCard));
        cmbTheme_ = new QComboBox(topCard);
        cmbTheme_->setToolTip(QStringLiteral("Debugger Theme — Changes the Live Debugger color palette and persists your choice."));
        for (const QString& tn : allThemeNames(false)) cmbTheme_->addItem(tn);
        cmbTheme_->setCurrentText(themeName_);
        connect(cmbTheme_, &QComboBox::activated, this, [this](int idx) {
            const QString nm = cmbTheme_->itemText(idx);
            if (nm != QLatin1String(CUSTOM_THEME_NAME)) {
                themeName_ = nm;
                theme_ = themeByName(nm);
                updateGuiSettingsPatch(QJsonObject{{QStringLiteral("theme"), nm}});
                applyThemeAndFont();
            }
        });
        topLay->addWidget(cmbTheme_);

        fontBar_ = new FontScaleBar(fontPct_, false, topCard);
        connect(fontBar_, &FontScaleBar::fontScaleChanged, this, [this](int p) { setFontPct(p); });
        topLay->addWidget(fontBar_);
        root->addWidget(topCard);

        dbgWidget_ = new SalsaLiveDebuggerWidget(initialTab, central);
        root->addWidget(dbgWidget_, 1);

        // Keyboard shortcuts: Ctrl+1..5 tabs, F5 refresh, Ctrl++/-/=/0 zoom, Ctrl+Q quit
        for (int t = 0; t < 5; ++t) {
            auto* sc = new QShortcut(QKeySequence(QStringLiteral("Ctrl+%1").arg(t + 1)), this);
            connect(sc, &QShortcut::activated, this, [this, t]() { dbgWidget_->selectTab(t); });
        }
        auto* scRef = new QShortcut(QKeySequence(Qt::Key_F5), this);
        connect(scRef, &QShortcut::activated, this, [this]() { dbgWidget_->refreshAll(); });
        for (const QKeySequence& seq : {QKeySequence(QKeySequence::ZoomIn), QKeySequence(Qt::CTRL | Qt::Key_Equal)}) {
            auto* scIn = new QShortcut(seq, this);
            connect(scIn, &QShortcut::activated, this, [this]() { setFontPct(fontPct_ + 10); });
        }
        for (const QKeySequence& seq : {QKeySequence(QKeySequence::ZoomOut), QKeySequence(Qt::CTRL | Qt::Key_Minus)}) {
            auto* scOut = new QShortcut(seq, this);
            connect(scOut, &QShortcut::activated, this, [this]() { setFontPct(fontPct_ - 10); });
        }
        auto* scRst = new QShortcut(QKeySequence(QStringLiteral("Ctrl+0")), this);
        connect(scRst, &QShortcut::activated, this, [this]() { setFontPct(100); });
        auto* scQuit = new QShortcut(QKeySequence(QStringLiteral("Ctrl+Q")), this);
        connect(scQuit, &QShortcut::activated, this, &QWidget::close);

        syncTimer_ = new QTimer(this);
        syncTimer_->setInterval(1000);
        connect(syncTimer_, &QTimer::timeout, this, &SalsaDebugWindow::checkSettingsSync);
        syncTimer_->start();

        applyThemeAndFont();
    }

    void setScreenCastLive(bool enabled) {
        if (dbgWidget_) dbgWidget_->setScreenCastLive(enabled);
    }

protected:
    void closeEvent(QCloseEvent* event) override {
        if (syncTimer_) syncTimer_->stop();
        if (dbgWidget_) dbgWidget_->shutdown();
        QMainWindow::closeEvent(event);
    }

private:
    void setFontPct(int pct) {
        fontPct_ = normalizeFontScalePct(pct);
        salsa_save_font_scale(fontPct_);
        lastMtimeNs_ = salsa_gui_settings_mtime_ns();
        applyThemeAndFont();
    }

    void applyThemeAndFont() {
        auto* app = qobject_cast<QApplication*>(QApplication::instance());
        if (app) {
            applyGlobalFontScale(app, fontPct_);
            app->setPalette(buildSalsaPalette(theme_));
            app->setStyleSheet(buildSalsaStylesheet(theme_, false, fontPct_));
        }
        if (fontBar_) fontBar_->setFontScalePct(fontPct_);
        if (dbgWidget_) dbgWidget_->setTheme(theme_, fontPct_);
    }

    void checkSettingsSync() {
        const uint64_t mt = salsa_gui_settings_mtime_ns();
        if (mt != 0 && mt != lastMtimeNs_) {
            lastMtimeNs_ = mt;
            guiSettings_ = loadGuiSettings();
            const int dp = normalizeFontScalePct(salsa_load_font_scale());
            const QString newT = guiSettings_.value(QStringLiteral("theme")).toString(themeName_);
            bool changed = false;
            if (dp != fontPct_) {
                fontPct_ = dp;
                changed = true;
            }
            if (!newT.isEmpty() && newT != themeName_) {
                themeName_ = newT;
                theme_ = loadThemeForApp(false);
                if (cmbTheme_) cmbTheme_->setCurrentText(themeName_);
                changed = true;
            }
            if (changed) applyThemeAndFont();
        }
    }

    QJsonObject guiSettings_;
    SalsaThemeData theme_;
    QString themeName_;
    int fontPct_ = 100;
    uint64_t lastMtimeNs_ = 0;

    QLabel* lblSub_ = nullptr;
    QComboBox* cmbTheme_ = nullptr;
    FontScaleBar* fontBar_ = nullptr;
    SalsaLiveDebuggerWidget* dbgWidget_ = nullptr;
    QTimer* syncTimer_ = nullptr;
};

}  // namespace salsa

#include "salsa_debug.moc"

static bool isCliModeArg(int argc, char** argv) {
    for (int i = 1; i < argc; ++i) {
        const char* a = argv[i];
        if (std::strcmp(a, "--snapshot") == 0
            || std::strcmp(a, "--stream") == 0
            || std::strcmp(a, "--cli") == 0
            || std::strcmp(a, "--audit") == 0
            || std::strcmp(a, "--probe") == 0
            || std::strcmp(a, "--probes") == 0
            || std::strcmp(a, "--tone") == 0
            || std::strcmp(a, "--midi") == 0
            || std::strcmp(a, "--export") == 0
            || std::strcmp(a, "--export-text") == 0
            || std::strcmp(a, "--text") == 0
            || std::strcmp(a, "--journal") == 0
            || std::strcmp(a, "--autosync") == 0
            || std::strcmp(a, "--dbus") == 0
            || std::strcmp(a, "--pw-test") == 0
            || std::strcmp(a, "--send") == 0
            || std::strcmp(a, "-V") == 0
            || std::strcmp(a, "--version") == 0
            || std::strcmp(a, "--license") == 0
            || std::strcmp(a, "--legal") == 0
            || std::strcmp(a, "-h") == 0
            || std::strcmp(a, "--help") == 0) {
            return true;
        }
    }
    return false;
}

int main(int argc, char** argv) {
    salsa_reset_sigpipe();

    bool selfTest = false;
    bool liveFeed = false;
    int initialTab = 0;
    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--self-test") == 0) selfTest = true;
        else if (std::strcmp(argv[i], "--live-feed") == 0 || std::strcmp(argv[i], "--live") == 0) liveFeed = true;
        else if (std::strcmp(argv[i], "--tab") == 0 && i + 1 < argc) {
            initialTab = std::clamp(std::atoi(argv[++i]), 0, 4);
        }
    }

    if (!salsa::verifyBridgeAbi()) {
        std::fprintf(stderr, "FATAL: Salsa ABI struct size mismatch between C++ and Rust\n");
        return 2;
    }

    if (selfTest) {
        qputenv("QT_QPA_PLATFORM", "offscreen");
        QApplication app(argc, argv);
        const salsa::SalsaStr st(salsa_debug_selftest());
        const QString res = st.toQString();
        if (!res.startsWith(QLatin1String("OK:"))) {
            std::fprintf(stderr, "%s\n", res.toUtf8().constData());
            return 1;
        }
        salsa::SalsaDebugWindow win(0);
        std::printf("%s\n", res.toUtf8().constData());
        return 0;
    }

    if (isCliModeArg(argc, argv)) {
        return salsa_debug_cli_main(argc, argv);
    }

    QApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("salsa-debug"));
    app.setDesktopFileName(QStringLiteral("salsa-debug"));

    salsa::SalsaDebugWindow win(initialTab);
    win.show();
    if (liveFeed) {
        QTimer::singleShot(150, &win, [&win]() { win.setScreenCastLive(true); });
    }
    return app.exec();
}

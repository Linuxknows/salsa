// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#include "salsa_common.hpp"
#include "salsa_widgets.hpp"

#include <QAbstractItemView>
#include <QApplication>
#include <QCheckBox>
#include <QCloseEvent>
#include <QComboBox>
#include <QDesktopServices>
#include <QDialog>
#include <QDir>
#include <QDoubleSpinBox>
#include <QFileDialog>
#include <QFileInfo>
#include <QFrame>
#include <QGridLayout>
#include <QGuiApplication>
#include <QHBoxLayout>
#include <QHeaderView>
#include <QKeyEvent>
#include <QKeySequence>
#include <QLabel>
#include <QLineEdit>
#include <QMainWindow>
#include <QMessageBox>
#include <QMetaObject>
#include <QPlainTextEdit>
#include <QPointer>
#include <QPushButton>
#include <QScreen>
#include <QScrollArea>
#include <QShortcut>
#include <QSlider>
#include <QSpinBox>
#include <QSplitter>
#include <QTableWidget>
#include <QTextBrowser>
#include <QThread>
#include <QTimer>
#include <QUrl>
#include <QVBoxLayout>
#include <QWindow>

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>

namespace {

inline QString formatBoostDbText(double db) {
    return QStringLiteral("%1 dB").arg(QStringLiteral("+%1").arg(db, 0, 'f', 1), 5);
}

inline QString formatPctText(int pct) {
    return QStringLiteral("%1%").arg(pct, 3);
}

}  // namespace

namespace salsa {

inline QString dynModeSavedLabel(uint32_t m) {
    switch (m) {
        case 0: return QStringLiteral("Saved: Static Parametric EQ");
        case 2: return QStringLiteral("Saved: Dynamic Expand");
        default: return QStringLiteral("Saved: Dynamic Compress");
    }
}

QString fmtFreqHz(float hz) {
    if (hz >= 1000.0f) return QStringLiteral("%1 kHz").arg(hz / 1000.0f, 0, 'g', 3);
    return QStringLiteral("%1 Hz").arg(static_cast<int>(std::lround(hz)));
}

class ShortcutCaptureEdit : public QLineEdit {
    Q_OBJECT
public:
    explicit ShortcutCaptureEdit(const QString& keyName, const QString& seq, QWidget* parent = nullptr)
        : QLineEdit(seq, parent), keyName_(keyName) {
        setReadOnly(true);
        setAlignment(Qt::AlignCenter);
        setToolTip(QStringLiteral("Click here and press any key combo; Backspace clears/restores"));
    }

signals:
    void shortcutCaptured(const QString& actionKey, const QString& seq);

protected:
    void keyPressEvent(QKeyEvent* event) override {
        const int key = event->key();
        if (key == Qt::Key_Control || key == Qt::Key_Shift || key == Qt::Key_Alt || key == Qt::Key_Meta || key == Qt::Key_unknown) {
            return;
        }
        const Qt::KeyboardModifiers mods = event->modifiers() & (Qt::ControlModifier | Qt::ShiftModifier | Qt::AltModifier | Qt::MetaModifier);
        const QKeySequence ks(static_cast<int>(mods.toInt()) | key);
        const QString txt = ks.toString(QKeySequence::PortableText);
        if (!txt.isEmpty()) {
            setText(txt);
            emit shortcutCaptured(keyName_, txt);
        }
        event->accept();
    }

private:
    QString keyName_;
};

class StudioController : public QObject {
    Q_OBJECT
public:
    explicit StudioController(bool forceStudio = false, QObject* parent = nullptr);
    ~StudioController() override;

    void start(
        bool forceStudio,
        bool forceTopology = false,
        bool forceAdvanced = false,
        bool forceThemeStudio = false,
        bool forceAbout = false);
    void dumpAuditPngs(const QString& outDir);
    void shutdownAll();

private:
    void buildMainWindow();
    void buildTopologyWindow();
    void buildAdvancedWindow();

    void applyThemeAndFontAll(bool saveSettings = false);
    void setFontScalePctAndSync(int pct);
    void syncFontScaleBars();
    void applyDynamicShortcuts();
    void populatePresetDropdown(const QString& selectTarget = QString());

    void pollEngine(bool initial);
    void checkSettingsFileSync();
    void syncGraphTimer();

    void pushEqToEngine();
    void saveStudioSettings();
    void applyGuiSettingsJsonToUi(bool isReload);
    void flashSaveFeedback(const QString& statusMsg, bool ok);
    void onSaveOverwriteConfigClicked();
    void onReloadConfigClicked();
    void onOpenConfigFileClicked();
    void updateSpectrumFromState();
    void updateBandParamTable();
    void refreshAdvIntegration();

    // Event handlers
    void onThemeActivated(int index);
    void openCustomThemeDialog();
    void onClockChanged();
    void onCapRateChanged();
    void onPresetActivated(int index);
    void onResetFlatClicked();
    void onImportAutoEqClicked();
    void onReloadAutoEqClicked();
    void onDynamicsSliderChanged();
    void onBandSliderChanged(int band, int tenths);
    void onPerDeviceEqToggled(bool on);
    void savePerDeviceEqForSink(const QString& sinkId);
    bool loadPerDeviceEqForSink(const QString& sinkId);

    void onSinkActivated(int idx);
    void onSourceActivated(int idx);
    void onBtResetClicked();
    void onBtCfgChanged();
    void onHwAnalogCfgChanged();
    void scheduleHwAnalogSend(bool immediate = false);
    void onHwAnalogReset();

    void onVolStepChanged();
    void onSpkSliderMoved(int v);
    void onMicSliderMoved(int v);
    void sendSpkVolNow(int muteOverride = -1);
    void sendMicVolNow(int muteOverride = -1);
    void onSessionSliderMoved(uint32_t sid, int v);
    void flushSessionVolume(uint32_t sid, int v);

    void openTopologyWindow();
    void enterStudioFromTopology();
    void openAdvancedWindow();
    void openAboutWindow(bool standaloneQuit = false);
    void selectManualChapter(const QString& key);
    void openLiveDebuggerWindow();

    void runSystemdRestart(const QString& unit, QPushButton* btn);

    bool forceStudio_ = false;
    bool closingAll_ = false;
    bool studioEntered_ = false;
    bool updatingUi_ = false;
    int fontPct_ = 100;
    int volStepPct_ = 5;
    uint64_t lastSettingsMtimeNs_ = 0;

    QJsonObject guiSettings_;
    QJsonObject hostProf_;
    SalsaThemeData currentTheme_;
    QString currentThemeName_;
    QJsonArray builtinEqPresets_;
    QJsonArray savedAutoEq_;
    QMap<QString, QString> shortcutsMap_;

    SalsaEngineStatus status_{};
    SalsaDeviceList devices_{};
    SalsaEqConfig eqCfg_{};
    SalsaParamEqExt paramExt_{};
    SalsaBtCodecCfg btCfg_{};
    SalsaHwAnalogCfg hwAnalog_{};
    bool hasStatus_ = false;
    QString lastSinkId_;
    QString targetPbId_ = QStringLiteral("auto");
    QString targetCapId_ = QStringLiteral("auto");
    QString lastDevSig_;
    QString currentManualKey_ = QStringLiteral("overview");

    qint64 eqHoldUntilMs_ = 0;
    qint64 btHoldUntilMs_ = 0;
    qint64 hwHoldUntilMs_ = 0;
    qint64 spkHoldUntilMs_ = 0;
    qint64 micHoldUntilMs_ = 0;
    qint64 lastDevPollMs_ = 0;
    qint64 lastSettingsPollMs_ = 0;

    // 3 Top-level windows + theme studio dialog
    QMainWindow* mainWin_ = nullptr;
    QWidget* introWin_ = nullptr;
    QWidget* advWin_ = nullptr;
    SalsaCustomThemeDialog* themeDlg_ = nullptr;

    // Main header widgets
    QLabel* lblTitle_ = nullptr;
    QLabel* lblVerPill_ = nullptr;
    QLabel* lblSubtitle_ = nullptr;
    QPushButton* btnSaveCfgMain_ = nullptr;
    QLabel* lblClockSrc_ = nullptr;
    QLabel* lblLatency_ = nullptr;
    QComboBox* cmbRate_ = nullptr;
    QComboBox* cmbPeriod_ = nullptr;
    QComboBox* cmbCapRate_ = nullptr;
    QComboBox* cmbBits_ = nullptr;
    QComboBox* cmbTheme_ = nullptr;
    FontScaleBar* fontBarMain_ = nullptr;
    FontScaleBar* fontBarIntro_ = nullptr;
    FontScaleBar* fontBarAdv_ = nullptr;

    // EQ widgets
    QCheckBox* chkEqEnable_ = nullptr;
    QLabel* lblSavedPreset_ = nullptr;
    QComboBox* cmbPreset_ = nullptr;
    QLabel* lblSavedDynMode_ = nullptr;
    QComboBox* cmbDynMode_ = nullptr;
    QPushButton* btnReloadAutoEq_ = nullptr;
    SalsaSpectrumCanvas* spectrum_ = nullptr;
    QLabel* lblPreamp_ = nullptr;
    QSlider* sldPreamp_ = nullptr;
    QLabel* lblThresh_ = nullptr;
    QSlider* sldThresh_ = nullptr;
    QLabel* lblRatio_ = nullptr;
    QSlider* sldRatio_ = nullptr;
    QLabel* lblPeaks_ = nullptr;
    std::array<QSlider*, SALSA_EQ_BANDS> bandSliders_{};
    std::array<QLabel*, SALSA_EQ_BANDS> bandValLbls_{};
    std::array<QLabel*, SALSA_EQ_BANDS> bandDynLbls_{};
    std::array<QLabel*, SALSA_EQ_BANDS> bandFreqLbls_{};

    // Master Mixer + BT + Sessions
    QComboBox* cmbVolStep_ = nullptr;
    QLabel* lblEndpoints_ = nullptr;
    QComboBox* cmbSink_ = nullptr;
    QComboBox* cmbSource_ = nullptr;
    QPushButton* btnBtReset_ = nullptr;
    QComboBox* cmbBtCodec_ = nullptr;
    QComboBox* cmbBtLdac_ = nullptr;
    QCheckBox* chkBtAbr_ = nullptr;
    QCheckBox* chkBtSco_ = nullptr;
    QCheckBox* chkBtOptional_ = nullptr;
    QPushButton* btnBtLink_ = nullptr;
    uint32_t lastBtConnFlags_ = SALSA_BT_STAT_OPTIONAL;
    qint64 lastBtLinkPollMs_ = 0;
    QLabel* lblBtDiag_ = nullptr;
    SingleInstanceServer singleInstServer_;
    void setupSingleInstanceServer();
    void activateExistingWindow(const QString& targetMode, const QByteArray& actToken);
    void syncBtConnectionControls(uint32_t flags, const QString& noteOverride = QString());
    QLabel* lblSpk_ = nullptr;
    StepVolumeSlider* sldSpk_ = nullptr;
    QPushButton* btnSpkMute_ = nullptr;
    QLabel* lblMic_ = nullptr;
    StepVolumeSlider* sldMic_ = nullptr;
    QPushButton* btnMicMute_ = nullptr;
    QLabel* lblMidiStatus_ = nullptr;
    QLabel* lblSessHdr_ = nullptr;
    QTableWidget* tblSessions_ = nullptr;
    struct SessRowWidgets {
        uint32_t sid = 0;
        StepVolumeSlider* volSlider = nullptr;
        QPushButton* btnMute = nullptr;
        QComboBox* cmbRoute = nullptr;
        QTableWidgetItem* fmtItem = nullptr;
        qint64 holdUntilMs = 0;
        qint64 lastSentMs = 0;
    };
    std::vector<SessRowWidgets> sessRows_;

    // Topology Intro + Manual widgets
    SalsaAutoGraphCanvas* graphCanvas_ = nullptr;
    QMap<QString, QPushButton*> manualBtns_;
    QTextBrowser* manualBrowser_ = nullptr;
    QCheckBox* chkHideIntroStartup_ = nullptr;

    // Advanced Options widgets
    QPushButton* btnRstEngine_ = nullptr;
    QPushButton* btnRstTray_ = nullptr;
    QLabel* lblChipSteam_ = nullptr;
    QLabel* lblChipLaunchers_ = nullptr;
    QLabel* lblChipPacman_ = nullptr;
    QPushButton* btnTogglePacman_ = nullptr;
    QCheckBox* chkAutosyncNow_ = nullptr;
    QPushButton* btnEngineToggle_ = nullptr;
    QCheckBox* chkShowIntroAdv_ = nullptr;
    QCheckBox* chkLimiter_ = nullptr;
    QCheckBox* chkEqPerDev_ = nullptr;
    QCheckBox* chkCrossfeed_ = nullptr;
    QSlider* sldCrossfeed_ = nullptr;
    QLabel* lblCrossfeed_ = nullptr;
    QComboBox* cmbCrossfeedCut_ = nullptr;
    QTableWidget* tblParamBands_ = nullptr;
    QCheckBox* chkHwDacUnity_ = nullptr;
    QComboBox* cmbHwAmp_ = nullptr;
    QComboBox* cmbHwRoute_ = nullptr;
    QCheckBox* chkHwRails_ = nullptr;
    QSlider* sldHwBoost_ = nullptr;
    QLabel* lblHwBoost_ = nullptr;
    QSlider* sldHwMicBoost_ = nullptr;
    QLabel* lblHwMicBoost_ = nullptr;
    QPushButton* btnSaveHw_ = nullptr;
    QLabel* lblHwAnalogDiag_ = nullptr;
    QPushButton* btnSaveCfgAdv_ = nullptr;
    QPushButton* btnReloadCfg_ = nullptr;
    QPushButton* btnOpenCfgAdv_ = nullptr;
    QLabel* lblCfgSaveStatus_ = nullptr;
    QMap<QString, ShortcutCaptureEdit*> shortcutEdits_;
    std::vector<QShortcut*> activeShortcuts_;

    // Timers
    QTimer* pollTimer_ = nullptr;
    QTimer* graphAnimTimer_ = nullptr;
    QTimer* spkSendTimer_ = nullptr;
    QTimer* micSendTimer_ = nullptr;
    QTimer* hwSendTimer_ = nullptr;
    QTimer* dbgSnapTimer_ = nullptr;
    QTimer* settingsSaveTimer_ = nullptr;
    void* studioDbgHandle_ = nullptr;
};

class StudioMainWindow : public QMainWindow {
public:
    explicit StudioMainWindow(std::function<bool(QCloseEvent*)> onClose)
        : onClose_(std::move(onClose)) {}

protected:
    void closeEvent(QCloseEvent* event) override {
        if (onClose_ && !onClose_(event)) {
            event->ignore();
            return;
        }
        QMainWindow::closeEvent(event);
    }

private:
    std::function<bool(QCloseEvent*)> onClose_;
};

class PopToolWindow : public QWidget {
public:
    explicit PopToolWindow(const QString& title, std::function<bool(QCloseEvent*)> onClose)
        : QWidget(nullptr), onClose_(std::move(onClose)) {
        setWindowTitle(title);
        setWindowFlag(Qt::Window, true);
    }

protected:
    void closeEvent(QCloseEvent* event) override {
        if (onClose_ && !onClose_(event)) {
            event->ignore();
            return;
        }
        QWidget::closeEvent(event);
    }
    void keyPressEvent(QKeyEvent* event) override {
        if (event->key() == Qt::Key_Escape) {
            close();
            event->accept();
            return;
        }
        QWidget::keyPressEvent(event);
    }

private:
    std::function<bool(QCloseEvent*)> onClose_;
};

static qint64 nowMs() {
    return QDateTime::currentMSecsSinceEpoch();
}

StudioController::StudioController(bool forceStudio, QObject* parent)
    : QObject(parent), forceStudio_(forceStudio) {
    guiSettings_ = loadGuiSettings();
    hostProf_ = hostProfile();
    fontPct_ = normalizeFontScalePct(salsa_load_font_scale());
    volStepPct_ = normalizeVolStepPct(guiSettings_.value(QStringLiteral("volume_step_pct")).toInt(5));
    lastSettingsMtimeNs_ = salsa_gui_settings_mtime_ns();

    currentTheme_ = loadThemeForApp(false);
    currentThemeName_ = !currentTheme_.name.isEmpty()
        ? currentTheme_.name
        : guiSettings_.value(QStringLiteral("theme")).toString(QStringLiteral("Salsa Midnight (Default)"));
    builtinEqPresets_ = parseJsonArr(SalsaStr(salsa_builtin_eq_presets_json()).toQString());
    savedAutoEq_ = parseJsonArr(SalsaStr(salsa_saved_autoeq_list_json()).toQString());

    const QJsonArray defSc = parseJsonArr(SalsaStr(salsa_default_shortcuts_json()).toQString());
    const QJsonObject savedSc = guiSettings_.value(QStringLiteral("shortcuts")).toObject();
    for (const QJsonValue& v : defSc) {
        const QJsonObject o = v.toObject();
        const QString k = o.value(QStringLiteral("key")).toString();
        const QString defSeq = o.value(QStringLiteral("default_seq")).toString();
        const QString chosen = savedSc.value(k).toString(defSeq);
        if (!k.isEmpty()) shortcutsMap_.insert(k, chosen);
    }

    // Initialize default EQ/ext structs cleanly before first poll
    eqCfg_.enabled = 1;
    eqCfg_.dynamic_mode = 1;
    eqCfg_.dyn_threshold_db = -18.0f;
    eqCfg_.dyn_ratio = 2.5f;
    eqCfg_.limiter_enabled = 1;
    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
        eqCfg_.band_q[i] = 1.414f;
        paramExt_.band_freq_hz[i] = DEFAULT_BAND_FREQS_HZ[i];
        paramExt_.band_type[i] = SALSA_FILTER_PEAK;
    }
    paramExt_.crossfeed_level = 0.30f;
    paramExt_.crossfeed_cutoff_hz = 700;

    buildMainWindow();
    buildTopologyWindow();
    buildAdvancedWindow();

    // Restore persisted settings (EQ, engine clock, volumes, HW analog, BT) immediately
    applyGuiSettingsJsonToUi(false);

    applyThemeAndFontAll(false);
    applyDynamicShortcuts();

    spkSendTimer_ = new QTimer(this);
    spkSendTimer_->setSingleShot(true);
    spkSendTimer_->setInterval(40);
    connect(spkSendTimer_, &QTimer::timeout, this, [this]() {
        sendSpkVolNow();
    });

    micSendTimer_ = new QTimer(this);
    micSendTimer_->setSingleShot(true);
    micSendTimer_->setInterval(40);
    connect(micSendTimer_, &QTimer::timeout, this, [this]() {
        sendMicVolNow();
    });

    hwSendTimer_ = new QTimer(this);
    hwSendTimer_->setSingleShot(true);
    hwSendTimer_->setInterval(40);
    connect(hwSendTimer_, &QTimer::timeout, this, [this]() {
        onHwAnalogCfgChanged();
    });

    pollTimer_ = new QTimer(this);
    pollTimer_->setTimerType(Qt::CoarseTimer);
    pollTimer_->setInterval(salsaTelemetryIntervalMs(mainWin_, 120));
    connect(pollTimer_, &QTimer::timeout, this, [this]() {
        pollEngine(false);
        const qint64 now = nowMs();
        if (now - lastSettingsPollMs_ >= 450) {
            lastSettingsPollMs_ = now;
            checkSettingsFileSync();
        }
    });

    settingsSaveTimer_ = new QTimer(this);
    settingsSaveTimer_->setSingleShot(true);
    settingsSaveTimer_->setInterval(250);
    connect(settingsSaveTimer_, &QTimer::timeout, this, [this]() {
        saveStudioSettings();
    });

    graphAnimTimer_ = new QTimer(this);
    graphAnimTimer_->setInterval(35);
    connect(graphAnimTimer_, &QTimer::timeout, this, [this]() {
        if (graphCanvas_ && introWin_ && introWin_->isVisible()) {
            graphCanvas_->advancePulse(0.035);
        }
    });

    dbgSnapTimer_ = new QTimer(this);
    dbgSnapTimer_->setInterval(1200);
    connect(dbgSnapTimer_, &QTimer::timeout, this, [this]() {
        if (introWin_ && introWin_->isVisible() && studioDbgHandle_) {
            salsa_dbg_request_snapshot(studioDbgHandle_);
        }
    });
}

StudioController::~StudioController() {
    shutdownAll();
}

void StudioController::shutdownAll() {
    closingAll_ = true;
    if (hwSendTimer_ && hwSendTimer_->isActive()) {
        hwSendTimer_->stop();
        onHwAnalogCfgChanged();
    }
    if (settingsSaveTimer_ && settingsSaveTimer_->isActive()) {
        settingsSaveTimer_->stop();
        saveStudioSettings();
    }
    if (pollTimer_) pollTimer_->stop();
    if (graphAnimTimer_) graphAnimTimer_->stop();
    if (dbgSnapTimer_) dbgSnapTimer_->stop();
    if (studioDbgHandle_) {
        salsa_dbg_destroy(studioDbgHandle_);
        studioDbgHandle_ = nullptr;
    }
    if (themeDlg_) { themeDlg_->close(); delete themeDlg_; themeDlg_ = nullptr; }
    singleInstServer_.stop();
    if (advWin_) { advWin_->close(); }
    if (introWin_) { introWin_->close(); }
    if (mainWin_) { mainWin_->close(); }
}

void StudioController::setupSingleInstanceServer() {
    singleInstServer_.start(
        QStringLiteral("salsa-studio.sock"),
        this,
        [this](const QString& mode, const QByteArray& token) {
            activateExistingWindow(mode, token);
        });
}

void StudioController::activateExistingWindow(const QString& targetMode, const QByteArray& actToken) {
    QWidget* target = nullptr;
    if (targetMode == QStringLiteral("advanced")) {
        openAdvancedWindow();
        target = advWin_;
    } else if (targetMode == QStringLiteral("theme-studio")) {
        openCustomThemeDialog();
        target = themeDlg_;
    } else if (targetMode == QStringLiteral("about")) {
        if (!actToken.isEmpty()) qputenv("XDG_ACTIVATION_TOKEN", actToken);
        openAboutWindow(false);
        return;
    } else if (targetMode == QStringLiteral("topology") && introWin_) {
        target = introWin_;
    } else if (mainWin_
               && (targetMode == QStringLiteral("studio")
                   || studioEntered_
                   || mainWin_->isVisible()
                   || mainWin_->isMinimized())) {
        studioEntered_ = true;
        target = mainWin_;
        if (targetMode == QStringLiteral("studio") && introWin_) {
            introWin_->hide();
        }
    } else if (introWin_) {
        target = introWin_;
    } else if (mainWin_) {
        studioEntered_ = true;
        target = mainWin_;
    }
    if (!target) return;

    raiseAndActivateWindow(target, actToken);
    syncGraphTimer();
}

void StudioController::syncBtConnectionControls(uint32_t flags, const QString& noteOverride) {
    lastBtConnFlags_ = flags;
    const bool opt = (flags & SALSA_BT_STAT_OPTIONAL) != 0;
    const bool conn = (flags & SALSA_BT_STAT_CONNECTED) != 0;
    const bool tp = (flags & SALSA_BT_STAT_TRANSPORT) != 0;
    const bool usbPri = (flags & SALSA_BT_STAT_USB_MATCHES_BT) != 0;
    if (chkBtOptional_) {
        chkBtOptional_->blockSignals(true);
        chkBtOptional_->setChecked(opt);
        chkBtOptional_->blockSignals(false);
    }
    if (btnBtLink_) {
        btnBtLink_->setText(conn ? QStringLiteral("Disconnect BT") : QStringLiteral("Turn On BT"));
        btnBtLink_->setToolTip(conn
            ? QStringLiteral("Disconnect Bluetooth audio headset (USB cable & Bluetooth mice/keyboards stay active).")
            : (usbPri ? QStringLiteral("Turn On Bluetooth Headset — Currently prioritized to USB cable; click to connect BT anyway.")
                      : QStringLiteral("Turn On Bluetooth Headset — Manually initiate BlueZ audio connection when Optional BT is enabled.")));
    }
    if (lblBtDiag_) {
        if (!noteOverride.isEmpty()) {
            lblBtDiag_->setText(noteOverride);
        } else if (!tp && usbPri) {
            lblBtDiag_->setText(conn
                ? QStringLiteral("BT: Standby (USB Priority Active)")
                : QStringLiteral("BT: Optional Off (USB Priority Active)"));
        } else if (!tp && !conn && opt) {
            lblBtDiag_->setText(QStringLiteral("BT: Optional Standby"));
        }
    }
}

void StudioController::start(
    bool forceStudio,
    bool forceTopology,
    bool forceAdvanced,
    bool forceThemeStudio,
    bool forceAbout) {
    forceStudio_ = forceStudio;
    pollEngine(true);
    // Honor show_topology_intro unless an explicit window mode was passed
    if (forceAdvanced) {
        openAdvancedWindow();
    } else if (forceThemeStudio) {
        openCustomThemeDialog();
    } else if (forceAbout) {
        QTimer::singleShot(0, this, [this]() { openAboutWindow(true); });
    } else {
        const bool showIntro = forceTopology
                               || (!forceStudio_ && guiSettings_.value(QStringLiteral("show_topology_intro")).toBool(true));
        if (showIntro) {
            studioDbgHandle_ = salsa_dbg_create();
            introWin_->show();
            introWin_->raise();
            introWin_->activateWindow();
            dbgSnapTimer_->start();
        } else {
            studioEntered_ = true;
            mainWin_->show();
            mainWin_->raise();
            mainWin_->activateWindow();
        }
    }
    setupSingleInstanceServer();
    pollTimer_->start();
    syncGraphTimer();
}

void StudioController::buildMainWindow() {
    mainWin_ = new StudioMainWindow([this](QCloseEvent*) {
        if (closingAll_) return true;
        closingAll_ = true;
        if (advWin_) advWin_->hide();
        if (introWin_) introWin_->hide();
        QApplication::quit();
        return true;
    });
    mainWin_->setWindowTitle(QStringLiteral("Salsa — Linux WASAPI Studio"));
    QSize scrSz = QGuiApplication::primaryScreen() ? QGuiApplication::primaryScreen()->availableSize() : QSize(1600, 960);
    mainWin_->resize(
        std::min(scrSz.width() - 32, scaledPx(1320, fontPct_)),
        std::min(scrSz.height() - 32, scaledPx(820, fontPct_)));

    auto* central = new QWidget(mainWin_);
    mainWin_->setCentralWidget(central);
    auto* root = new QVBoxLayout(central);
    root->setContentsMargins(10, 8, 10, 8);
    root->setSpacing(6);

    // Top Card
    auto* topCard = new QFrame(central);
    topCard->setObjectName(QStringLiteral("Card"));
    auto* topLay = new QHBoxLayout(topCard);
    topLay->setContentsMargins(12, 8, 12, 8);
    topLay->setSpacing(10);

    auto* titleCol = new QVBoxLayout();
    auto* titleRow = new QHBoxLayout();
    titleRow->setSpacing(8);
    lblTitle_ = new QLabel(QStringLiteral("SALSA — LINUX WASAPI STUDIO"), topCard);
    lblTitle_->setObjectName(QStringLiteral("TitleBanner"));
    titleRow->addWidget(lblTitle_);
    lblVerPill_ = new QLabel(SalsaStr(salsa_version_str()).toQString(), topCard);
    lblVerPill_->setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Fixed);
    titleRow->addWidget(lblVerPill_);
    titleRow->addStretch(1);
    titleCol->addLayout(titleRow);

    lblSubtitle_ = new QLabel(
        QStringLiteral("Fixed-Clock Lossless WaveRT • Hardware DAC/Amp Analog • Dynamic Studio EQ • Host: %1 (%2)")
            .arg(hostProf_.value(QStringLiteral("os")).toString(),
                 hostProf_.value(QStringLiteral("cpu")).toString()),
        topCard);
    lblSubtitle_->setObjectName(QStringLiteral("SubBanner"));
    lblSubtitle_->setWordWrap(true);
    lblSubtitle_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Minimum);
    titleCol->addWidget(lblSubtitle_);
    titleCol->addStretch(1);
    topLay->addLayout(titleCol, 2);

    auto* rateCol = new QVBoxLayout();
    rateCol->addWidget(new QLabel(QStringLiteral("Master Rate:"), topCard));
    cmbRate_ = new QComboBox(topCard);
    for (int r : {44100, 48000, 88200, 96000, 176400, 192000}) {
        const QString tag = r >= 176400 ? QStringLiteral(" (Ultra-HD)") : (r >= 88200 ? QStringLiteral(" (Hi-Res)") : QString());
        cmbRate_->addItem(QStringLiteral("%1 Hz%2").arg(QLocale().toString(r), tag), r);
    }
    cmbRate_->setCurrentIndex(1);
    cmbRate_->setToolTip(QStringLiteral("Master Sample Rate — Fixes the engine's internal WaveRT mixing and DAC clock rate; apps at other rates are resampled cleanly into it."));
    connect(cmbRate_, &QComboBox::currentIndexChanged, this, [this]() { onClockChanged(); });
    rateCol->addWidget(cmbRate_);
    lblClockSrc_ = new QLabel(QStringLiteral("Clock: 48kHz / 32-bit"), topCard);
    lblClockSrc_->setObjectName(QStringLiteral("DiagStatusLabel"));
    rateCol->addWidget(lblClockSrc_);
    rateCol->addStretch(1);
    topLay->addLayout(rateCol);

    auto* periodCol = new QVBoxLayout();
    periodCol->addWidget(new QLabel(QStringLiteral("Quantum & Mic Rate:"), topCard));
    cmbPeriod_ = new QComboBox(topCard);
    for (int p : {64, 128, 256, 512, 1024, 2048}) cmbPeriod_->addItem(QStringLiteral("%1 frames").arg(p), p);
    cmbPeriod_->setCurrentIndex(2);
    cmbPeriod_->setToolTip(QStringLiteral("Engine Quantum (Period Frames) — Audio frames per SCHED_FIFO mixing tick; smaller frames cut latency, larger frames increase stability."));
    connect(cmbPeriod_, &QComboBox::currentIndexChanged, this, [this]() { onClockChanged(); });
    periodCol->addWidget(cmbPeriod_);
    cmbCapRate_ = new QComboBox(topCard);
    cmbCapRate_->addItem(QStringLiteral("Mic: Auto (Master)"), 0);
    for (int r : {44100, 48000, 88200, 96000, 176400, 192000}) {
        cmbCapRate_->addItem(QStringLiteral("Mic: %1 Hz").arg(QLocale().toString(r)), r);
    }
    cmbCapRate_->setToolTip(QStringLiteral("Independent Mic Rate — Pins the ALSA capture clock separately from Master playback (useful for fixed 48 kHz interfaces while running 96/192 kHz DACs)."));
    connect(cmbCapRate_, &QComboBox::currentIndexChanged, this, [this]() { onCapRateChanged(); });
    periodCol->addWidget(cmbCapRate_);
    periodCol->addStretch(1);
    topLay->addLayout(periodCol);

    auto* bitsCol = new QVBoxLayout();
    bitsCol->addWidget(new QLabel(QStringLiteral("Bit Depth:"), topCard));
    cmbBits_ = new QComboBox(topCard);
    cmbBits_->addItem(QStringLiteral("32-Bit Float"), 32);
    cmbBits_->addItem(QStringLiteral("24-Bit PCM"), 24);
    cmbBits_->addItem(QStringLiteral("16-Bit PCM"), 16);
    cmbBits_->setToolTip(QStringLiteral("Output Bit Depth — Selects hardware PCM packing depth (or 32-bit float when supported) while DSP always runs in 32-bit float."));
    connect(cmbBits_, &QComboBox::currentIndexChanged, this, [this]() { onClockChanged(); });
    bitsCol->addWidget(cmbBits_);
    bitsCol->addStretch(1);
    topLay->addLayout(bitsCol);

    auto* themeCol = new QVBoxLayout();
    themeCol->addWidget(new QLabel(QStringLiteral("Theme:"), topCard));
    cmbTheme_ = new QComboBox(topCard);
    for (const QString& tn : allThemeNames(false)) cmbTheme_->addItem(tn);
    cmbTheme_->setCurrentText(currentThemeName_);
    cmbTheme_->setToolTip(QStringLiteral("UI Color Theme — Applies built-in palettes or your saved Custom Theme across all Salsa Studio windows and persists it."));
    connect(cmbTheme_, &QComboBox::activated, this, [this](int idx) { onThemeActivated(idx); });
    themeCol->addWidget(cmbTheme_);
    themeCol->addStretch(1);
    topLay->addLayout(themeCol);

    auto* engCol = new QVBoxLayout();
    auto* engTopRow = new QHBoxLayout();
    engTopRow->setSpacing(6);
    lblLatency_ = new QLabel(QStringLiteral("Quantum: 5.33 ms | SCHED_FIFO"), topCard);
    lblLatency_->setObjectName(QStringLiteral("LatencyLabel"));
    engTopRow->addWidget(lblLatency_, 1);
    fontBarMain_ = new FontScaleBar(fontPct_, false, topCard);
    connect(fontBarMain_, &FontScaleBar::fontScaleChanged, this, [this](int p) { setFontScalePctAndSync(p); });
    engTopRow->addWidget(fontBarMain_);
    engCol->addLayout(engTopRow);

    auto* hdrBtns = new QGridLayout();
    hdrBtns->setHorizontalSpacing(6);
    hdrBtns->setVerticalSpacing(4);
    auto* btnTopo = new QPushButton(QStringLiteral("Topology (F1)"), topCard);
    btnTopo->setToolTip(QStringLiteral("Topology Graph & Manual (F1) — Opens the live interactive signal-flow map and Owner's Manual chapters."));
    connect(btnTopo, &QPushButton::clicked, this, [this]() { openTopologyWindow(); });
    hdrBtns->addWidget(btnTopo, 0, 0);
    auto* btnAdv = new QPushButton(QStringLiteral("Advanced Options (Ctrl+,)"), topCard);
    btnAdv->setObjectName(QStringLiteral("PrimaryBtn"));
    btnAdv->setToolTip(QStringLiteral("Advanced Options (Ctrl+,) — Opens parametric EQ table, crossfeed, discrete analog stages, shortcuts, and diagnostics."));
    connect(btnAdv, &QPushButton::clicked, this, [this]() { openAdvancedWindow(); });
    hdrBtns->addWidget(btnAdv, 0, 1);
    btnSaveCfgMain_ = new QPushButton(QStringLiteral("Save Settings (Ctrl+S)"), topCard);
    btnSaveCfgMain_->setObjectName(QStringLiteral("CompactBtn"));
    btnSaveCfgMain_->setToolTip(QStringLiteral("Save Settings (Ctrl+S) — Validates and saves all current engine, analog, BT, EQ, volume, and GUI settings to ~/.config/lwasapi/salsa_settings.json."));
    connect(btnSaveCfgMain_, &QPushButton::clicked, this, [this]() { onSaveOverwriteConfigClicked(); });
    hdrBtns->addWidget(btnSaveCfgMain_, 1, 0);
    auto* btnAboutMain = new QPushButton(QStringLiteral("About / LGPL-3.0"), topCard);
    btnAboutMain->setToolTip(QStringLiteral("About & LGPL-3.0 License — Displays Salsa copyright, no-warranty disclaimer, relinking/source terms, third-party dlopen disclosures, and full COPYING/COPYING.LESSER texts."));
    connect(btnAboutMain, &QPushButton::clicked, this, [this]() {
        showAboutLicenseDialog(mainWin_, QStringLiteral("Salsa Studio"), currentTheme_, fontPct_);
    });
    hdrBtns->addWidget(btnAboutMain, 1, 1);
    engCol->addLayout(hdrBtns);
    topLay->addLayout(engCol);
    root->addWidget(topCard);

    // Scrollable main content
    auto* scroll = new QScrollArea(central);
    scroll->setWidgetResizable(true);
    scroll->setFrameShape(QFrame::NoFrame);
    scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    auto* tabStudio = new QWidget(scroll);
    auto* studioLay = new QVBoxLayout(tabStudio);
    studioLay->setContentsMargins(0, 2, 0, 0);
    studioLay->setSpacing(6);

    // 1. EQ Card
    auto* eqCard = new QFrame(tabStudio);
    eqCard->setObjectName(QStringLiteral("Card"));
    auto* eqLay = new QVBoxLayout(eqCard);
    eqLay->setContentsMargins(10, 5, 10, 5);
    eqLay->setSpacing(4);

    auto* eqBar = new QHBoxLayout();
    eqBar->setSpacing(6);
    chkEqEnable_ = new QCheckBox(QStringLiteral("Enable Salsa 10-Band Dynamic EQ APO"), eqCard);
    chkEqEnable_->setChecked(true);
    chkEqEnable_->setObjectName(QStringLiteral("EqEnableCheck"));
    chkEqEnable_->setToolTip(QStringLiteral("Master EQ APO Enable — Toggles real-time 10-band biquad Parametric & Dynamic EQ processing on the engine output mix."));
    connect(chkEqEnable_, &QCheckBox::toggled, this, [this]() { if (!updatingUi_) pushEqToEngine(); });
    eqBar->addWidget(chkEqEnable_);

    eqBar->addStretch(1);
    eqBar->addWidget(new QLabel(QStringLiteral("Dynamic Mode:"), eqCard));
    lblSavedDynMode_ = new QLabel(QStringLiteral("Saved: Dynamic Compress"), eqCard);
    lblSavedDynMode_->setObjectName(QStringLiteral("SavedChipLabel"));
    eqBar->addWidget(lblSavedDynMode_);

    cmbDynMode_ = new QComboBox(eqCard);
    cmbDynMode_->addItem(QStringLiteral("Static Parametric EQ"), 0);
    cmbDynMode_->addItem(QStringLiteral("Dynamic Compress (Tame Peaks)"), 1);
    cmbDynMode_->addItem(QStringLiteral("Dynamic Expand (Upward Punch)"), 2);
    cmbDynMode_->setToolTip(QStringLiteral("Dynamic EQ Mode — Chooses Static fixed curves, Dynamic Compress (tames loud band peaks), or Dynamic Expand (adds punch above threshold)."));
    connect(cmbDynMode_, &QComboBox::currentIndexChanged, this, [this]() {
        if (!updatingUi_) {
            eqCfg_.dynamic_mode = static_cast<uint32_t>(qvarInt(cmbDynMode_->currentData(), 1));
            lblSavedDynMode_->setText(dynModeSavedLabel(eqCfg_.dynamic_mode));
            pushEqToEngine();
        }
    });
    eqBar->addWidget(cmbDynMode_);

    auto* btnFlat = new QPushButton(QStringLiteral("Reset Flat"), eqCard);
    btnFlat->setObjectName(QStringLiteral("CompactBtn"));
    btnFlat->setToolTip(QStringLiteral("Reset EQ Flat — Resets all 10 band gains to 0 dB, resets preamp to 0 dB, and restores standard band frequencies and Q."));
    connect(btnFlat, &QPushButton::clicked, this, [this]() { onResetFlatClicked(); });
    eqBar->addWidget(btnFlat);
    eqLay->addLayout(eqBar);

    auto* eqPresetRow = new QHBoxLayout();
    eqPresetRow->setSpacing(6);
    eqPresetRow->addWidget(new QLabel(QStringLiteral("Preset / AutoEQ:"), eqCard));
    lblSavedPreset_ = new QLabel(QStringLiteral("Saved: Flat / Studio Reference"), eqCard);
    lblSavedPreset_->setObjectName(QStringLiteral("SavedChipLabel"));
    eqPresetRow->addWidget(lblSavedPreset_);

    cmbPreset_ = new QComboBox(eqCard);
    cmbPreset_->setMinimumWidth(210);
    cmbPreset_->setToolTip(QStringLiteral("EQ Preset & AutoEQ Profile — Loads built-in target curves or imported AutoEQ files and recomputes DSP biquads immediately."));
    connect(cmbPreset_, &QComboBox::activated, this, [this](int idx) { onPresetActivated(idx); });
    eqPresetRow->addWidget(cmbPreset_);

    auto* btnImportEq = new QPushButton(QStringLiteral("Import AutoEQ .txt"), eqCard);
    btnImportEq->setObjectName(QStringLiteral("CompactBtn"));
    btnImportEq->setToolTip(QStringLiteral("Import & save EqualizerAPO / AutoEQ / Oratory1990 .txt profile"));
    connect(btnImportEq, &QPushButton::clicked, this, [this]() { onImportAutoEqClicked(); });
    eqPresetRow->addWidget(btnImportEq);

    btnReloadAutoEq_ = new QPushButton(QStringLiteral("Reload .txt"), eqCard);
    btnReloadAutoEq_->setObjectName(QStringLiteral("CompactBtn"));
    btnReloadAutoEq_->setToolTip(QStringLiteral("Reload active AutoEQ profile directly from its saved .txt file"));
    connect(btnReloadAutoEq_, &QPushButton::clicked, this, [this]() { onReloadAutoEqClicked(); });
    eqPresetRow->addWidget(btnReloadAutoEq_);
    eqPresetRow->addStretch(1);
    eqLay->addLayout(eqPresetRow);

    spectrum_ = new SalsaSpectrumCanvas(eqCard);
    connect(spectrum_, &SalsaSpectrumCanvas::bandGainDragged, this, [this](int band, double db) {
        updatingUi_ = true;
        bandSliders_[band]->setValue(static_cast<int>(std::lround(db * 10.0)));
        bandValLbls_[band]->setText(QStringLiteral("%1%2 dB").arg(db >= 0 ? QStringLiteral("+") : QString()).arg(db, 0, 'f', 1));
        updatingUi_ = false;
        eqCfg_.band_gains_db[band] = static_cast<float>(db);
        salsa_set_eq_preset_name(&eqCfg_, "Custom");
        lblSavedPreset_->setText(QStringLiteral("Saved: Custom"));
        pushEqToEngine();
    });
    connect(spectrum_, &SalsaSpectrumCanvas::bandQChanged, this, [this](int band, double q) {
        paramExt_.band_freq_hz[band] = std::max(15.0f, paramExt_.band_freq_hz[band]);
        eqCfg_.band_q[band] = static_cast<float>(q);
        updateBandParamTable();
        pushEqToEngine();
    });
    connect(spectrum_, &SalsaSpectrumCanvas::bandResetRequested, this, [this](int band) {
        bandSliders_[band]->setValue(0);
    });
    eqLay->addWidget(spectrum_);

    auto* dynRow = new QHBoxLayout();
    lblPreamp_ = new QLabel(QStringLiteral("Preamp: +0.0 dB"), eqCard);
    dynRow->addWidget(lblPreamp_);
    sldPreamp_ = new QSlider(Qt::Horizontal, eqCard);
    sldPreamp_->setRange(-180, 120);
    sldPreamp_->setToolTip(QStringLiteral("EQ Preamp Gain — Attenuates or boosts overall signal (-18 to +12 dB) before filtering to keep boosted EQ curves below 0 dBFS clipping."));
    connect(sldPreamp_, &QSlider::valueChanged, this, [this]() { onDynamicsSliderChanged(); });
    dynRow->addWidget(sldPreamp_);

    dynRow->addSpacing(12);
    lblThresh_ = new QLabel(QStringLiteral("Dyn Threshold: -18.0 dB"), eqCard);
    dynRow->addWidget(lblThresh_);
    sldThresh_ = new QSlider(Qt::Horizontal, eqCard);
    sldThresh_->setRange(-480, 0);
    sldThresh_->setValue(-180);
    sldThresh_->setToolTip(QStringLiteral("Dynamic EQ Threshold — Level (-48 to 0 dB) above which per-band dynamic compression or expansion starts tracking audio energy."));
    connect(sldThresh_, &QSlider::valueChanged, this, [this]() { onDynamicsSliderChanged(); });
    dynRow->addWidget(sldThresh_);

    dynRow->addSpacing(12);
    lblRatio_ = new QLabel(QStringLiteral("Dyn Ratio: 2.5:1"), eqCard);
    dynRow->addWidget(lblRatio_);
    sldRatio_ = new QSlider(Qt::Horizontal, eqCard);
    sldRatio_->setRange(10, 80);
    sldRatio_->setValue(25);
    sldRatio_->setToolTip(QStringLiteral("Dynamic EQ Ratio — Intensity (1.0:1 to 8.0:1) controlling how aggressively dynamic band gain scales past Threshold."));
    connect(sldRatio_, &QSlider::valueChanged, this, [this]() { onDynamicsSliderChanged(); });
    dynRow->addWidget(sldRatio_);

    dynRow->addSpacing(16);
    lblPeaks_ = new QLabel(QStringLiteral("Output Peak L/R: -100.0 / -100.0 dBFS"), eqCard);
    lblPeaks_->setObjectName(QStringLiteral("PeaksLabel"));
    lblPeaks_->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
    lblPeaks_->setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Preferred);
    dynRow->addWidget(lblPeaks_);
    eqLay->addLayout(dynRow);

    auto* fadersGrid = new QGridLayout();
    fadersGrid->setSpacing(6);
    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
        auto* colBox = new QVBoxLayout();
        colBox->setSpacing(2);
        auto* lv = new QLabel(QStringLiteral("+0.0 dB"), eqCard);
        lv->setAlignment(Qt::AlignCenter);
        lv->setObjectName(QStringLiteral("BandValLabel"));
        colBox->addWidget(lv);

        auto* sld = new QSlider(Qt::Vertical, eqCard);
        sld->setRange(-180, 180);
        sld->setSingleStep(5);
        sld->setPageStep(10);
        sld->setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Expanding);
        sld->setMinimumHeight(scaledPx(112, fontPct_));
        sld->setToolTip(QStringLiteral("EQ Band %1 (%2) Gain — Cuts or boosts this band (-18 to +18 dB) via real-time biquad filtering; drag here or on the canvas handle above.")
                            .arg(i + 1)
                            .arg(QString::fromLatin1(BAND_FREQS[i])));
        connect(sld, &QSlider::valueChanged, this, [this, i](int v) { onBandSliderChanged(i, v); });
        colBox->addWidget(sld, 1, Qt::AlignHCenter);

        auto* ld = new QLabel(QStringLiteral("Δ 0.0dB"), eqCard);
        ld->setAlignment(Qt::AlignCenter);
        ld->setObjectName(QStringLiteral("BandDynLabel"));
        colBox->addWidget(ld);

        auto* lf = new QLabel(QString::fromLatin1(BAND_FREQS[i]), eqCard);
        lf->setAlignment(Qt::AlignCenter);
        lf->setObjectName(QStringLiteral("BandFreqLabel"));
        colBox->addWidget(lf);

        fadersGrid->addLayout(colBox, 0, i);
        bandValLbls_[i] = lv;
        bandSliders_[i] = sld;
        bandDynLbls_[i] = ld;
        bandFreqLbls_[i] = lf;
    }
    eqLay->addLayout(fadersGrid, 1);
    studioLay->addWidget(eqCard, 4);

    // 2. Bottom Mixer & Sessions Row
    auto* botLay = new QHBoxLayout();
    botLay->setSpacing(10);

    auto* hwCard = new QFrame(tabStudio);
    hwCard->setObjectName(QStringLiteral("Card"));
    auto* hwCol = new QVBoxLayout(hwCard);
    hwCol->setContentsMargins(10, 6, 10, 6);
    hwCol->setSpacing(3);

    auto* hwHdrRow = new QHBoxLayout();
    auto* lblHwHdr = new QLabel(QStringLiteral("FULL-DUPLEX MASTER MIXER & DAW MIDI HUB"), hwCard);
    lblHwHdr->setObjectName(QStringLiteral("SectionHeader"));
    hwHdrRow->addWidget(lblHwHdr);
    hwHdrRow->addStretch(1);
    hwHdrRow->addWidget(new QLabel(QStringLiteral("Wheel/Key Step:"), hwCard));
    cmbVolStep_ = new QComboBox(hwCard);
    for (int st : SUPPORTED_VOL_STEPS) cmbVolStep_->addItem(QStringLiteral("%1%").arg(st), st);
    if (int si = cmbVolStep_->findData(volStepPct_); si >= 0) cmbVolStep_->setCurrentIndex(si);
    cmbVolStep_->setToolTip(QStringLiteral("Wheel / Key Volume Step — Controls how many percentage points each mouse-wheel notch or arrow key moves volume sliders (synced with tray)."));
    connect(cmbVolStep_, &QComboBox::currentIndexChanged, this, [this]() { onVolStepChanged(); });
    hwHdrRow->addWidget(cmbVolStep_);
    hwCol->addLayout(hwHdrRow);

    lblEndpoints_ = new QLabel(QStringLiteral("Sink: Detecting... | Mic: Detecting... | PA v35: Initializing..."), hwCard);
    lblEndpoints_->setObjectName(QStringLiteral("EndpointsLabel"));
    lblEndpoints_->setWordWrap(true);
    lblEndpoints_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Minimum);
    hwCol->addWidget(lblEndpoints_);

    auto* devRow = new QHBoxLayout();
    devRow->addWidget(new QLabel(QStringLiteral("Output Sink:"), hwCard));
    cmbSink_ = new QComboBox(hwCard);
    cmbSink_->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
    cmbSink_->setMinimumContentsLength(14);
    cmbSink_->addItem(QStringLiteral("Auto-Priority (Dual USB > BT A2DP > USB > Analog > HDMI)"), QStringLiteral("auto"));
    cmbSink_->setToolTip(QStringLiteral("Playback Output Sink — Auto-Priority gives plugged USB cables priority over matching Bluetooth headsets, then BT A2DP, USB, Analog, and HDMI."));
    connect(cmbSink_, &QComboBox::activated, this, [this](int i) { onSinkActivated(i); });
    devRow->addWidget(cmbSink_, 1);
    btnBtReset_ = new QPushButton(QStringLiteral("Reset BT"), hwCard);
    btnBtReset_->setToolTip(QStringLiteral("Reset Bluetooth Link — Releases and re-acquires the BlueZ A2DP L2CAP socket to recover stalled or glitched wireless headsets."));
    connect(btnBtReset_, &QPushButton::clicked, this, [this]() { onBtResetClicked(); });
    devRow->addWidget(btnBtReset_);
    devRow->addWidget(new QLabel(QStringLiteral("Mic Input:"), hwCard));
    cmbSource_ = new QComboBox(hwCard);
    cmbSource_->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
    cmbSource_->setMinimumContentsLength(14);
    cmbSource_->addItem(QStringLiteral("Auto-Priority (USB Mic > HDA > Virtual)"), QStringLiteral("auto"));
    cmbSource_->setToolTip(QStringLiteral("Microphone Capture Source — Selects Auto-Priority or pins a dedicated ALSA capture device for recording clients."));
    connect(cmbSource_, &QComboBox::activated, this, [this](int i) { onSourceActivated(i); });
    devRow->addWidget(cmbSource_, 1);
    hwCol->addLayout(devRow);

    auto* btRow = new QHBoxLayout();
    btRow->addWidget(new QLabel(QStringLiteral("BT Codec:"), hwCard));
    cmbBtCodec_ = new QComboBox(hwCard);
    cmbBtCodec_->addItem(QStringLiteral("Auto (Best)"), SALSA_BT_PREF_AUTO);
    cmbBtCodec_->addItem(QStringLiteral("LDAC"), SALSA_BT_PREF_LDAC);
    cmbBtCodec_->addItem(QStringLiteral("aptX HD"), SALSA_BT_PREF_APTX_HD);
    cmbBtCodec_->addItem(QStringLiteral("aptX"), SALSA_BT_PREF_APTX);
    cmbBtCodec_->addItem(QStringLiteral("AAC"), SALSA_BT_PREF_AAC);
    cmbBtCodec_->addItem(QStringLiteral("SBC"), SALSA_BT_PREF_SBC);
    cmbBtCodec_->setToolTip(QStringLiteral("Bluetooth Preferred Codec — Requests LDAC, aptX HD, aptX, AAC, or SBC negotiation via BlueZ MediaEndpoint1 when supported by the headset."));
    connect(cmbBtCodec_, &QComboBox::currentIndexChanged, this, [this]() { if (!updatingUi_) onBtCfgChanged(); });
    btRow->addWidget(cmbBtCodec_);

    cmbBtLdac_ = new QComboBox(hwCard);
    cmbBtLdac_->addItem(QStringLiteral("LDAC ABR"), SALSA_LDAC_MODE_ABR);
    cmbBtLdac_->addItem(QStringLiteral("990 kbps"), SALSA_LDAC_MODE_990);
    cmbBtLdac_->addItem(QStringLiteral("660 kbps"), SALSA_LDAC_MODE_660);
    cmbBtLdac_->addItem(QStringLiteral("330 kbps"), SALSA_LDAC_MODE_330);
    cmbBtLdac_->setToolTip(QStringLiteral("LDAC Bitrate Tier — Chooses adaptive ABR or locks fixed 990 / 660 / 330 kbps transmission when an LDAC headset is connected."));
    connect(cmbBtLdac_, &QComboBox::currentIndexChanged, this, [this]() { if (!updatingUi_) onBtCfgChanged(); });
    btRow->addWidget(cmbBtLdac_);

    chkBtAbr_ = new QCheckBox(QStringLiteral("Wireless ABR"), hwCard);
    chkBtAbr_->setChecked(true);
    chkBtAbr_->setToolTip(QStringLiteral("Wireless Adaptive Bitrate (ABR) — Automatically steps down or restores LDAC/AAC bitrate when L2CAP socket congestion is detected."));
    connect(chkBtAbr_, &QCheckBox::toggled, this, [this]() { if (!updatingUi_) onBtCfgChanged(); });
    btRow->addWidget(chkBtAbr_);

    chkBtSco_ = new QCheckBox(QStringLiteral("HFP Mic"), hwCard);
    chkBtSco_->setToolTip(QStringLiteral("Bluetooth HFP Two-Way Mic — Enables Hands-Free SCO capture using the headset's built-in microphone (lowers downlink audio bandwidth while active)."));
    connect(chkBtSco_, &QCheckBox::toggled, this, [this]() { if (!updatingUi_) onBtCfgChanged(); });
    btRow->addWidget(chkBtSco_);

    chkBtOptional_ = new QCheckBox(QStringLiteral("Optional BT"), hwCard);
    chkBtOptional_->setChecked(true);
    chkBtOptional_->setToolTip(QStringLiteral("Optional Bluetooth Connection — When enabled, Bluetooth is not mandatory when a matching USB cable or local card is active, eliminating background BT/mouse interference."));
    connect(chkBtOptional_, &QCheckBox::toggled, this, [this](bool checked) {
        if (updatingUi_) return;
        btHoldUntilMs_ = nowMs() + 1800;
        uint32_t flags = 0;
        const int rc = salsa_bt_connection_ctl(
            checked ? SALSA_BT_CONN_SET_OPTIONAL : SALSA_BT_CONN_SET_AUTOCONN,
            "",
            &flags);
        syncBtConnectionControls(flags, rc == 0 ? (checked ? QStringLiteral("BT: Optional (USB-First)") : QStringLiteral("BT: Auto-Connect enabled")) : QString());
    });
    btRow->addWidget(chkBtOptional_);

    btnBtLink_ = new QPushButton(QStringLiteral("Turn On BT"), hwCard);
    btnBtLink_->setObjectName(QStringLiteral("CompactBtn"));
    btnBtLink_->setToolTip(QStringLiteral("Connect or Disconnect Bluetooth Headset — Manually turn on or drop the BlueZ audio link without affecting Bluetooth mice or keyboards."));
    connect(btnBtLink_, &QPushButton::clicked, this, [this]() {
        btnBtLink_->setEnabled(false);
        btHoldUntilMs_ = nowMs() + 2500;
        const bool wantOff = (lastBtConnFlags_ & SALSA_BT_STAT_CONNECTED) != 0;
        uint32_t flags = 0;
        const int rc = salsa_bt_connection_ctl(
            wantOff ? SALSA_BT_CONN_DISCONNECT : SALSA_BT_CONN_CONNECT,
            "",
            &flags);
        btnBtLink_->setEnabled(true);
        QString note;
        if (rc == 0) {
            note = wantOff ? QStringLiteral("BT: Disconnected (USB/mice unaffected)") : QStringLiteral("BT: Connection requested");
        } else {
            note = wantOff ? QStringLiteral("BT: Disconnect failed") : QStringLiteral("BT: No paired audio headset ready");
        }
        syncBtConnectionControls(flags, note);
        lastBtLinkPollMs_ = 0;
    });
    btRow->addWidget(btnBtLink_);
    btRow->addStretch(1);
    hwCol->addLayout(btRow);

    lblBtDiag_ = new QLabel(QStringLiteral("BT: Standby"), hwCard);
    lblBtDiag_->setObjectName(QStringLiteral("DiagStatusLabel"));
    lblBtDiag_->setWordWrap(true);
    lblBtDiag_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Minimum);
    hwCol->addWidget(lblBtDiag_);

    auto* spkRow = new QHBoxLayout();
    lblSpk_ = new QLabel(QStringLiteral("Speaker Vol (80%):"), hwCard);
    lblSpk_->setMinimumWidth(135);
    spkRow->addWidget(lblSpk_);
    sldSpk_ = new StepVolumeSlider([this]() { return volStepPct_; }, Qt::Horizontal, hwCard);
    sldSpk_->setRange(0, 150);
    sldSpk_->setValue(80);
    sldSpk_->setToolTip(QStringLiteral("Master Speaker Volume — Adjusts global output gain (0–150%, syncs AVRCP on Bluetooth and DAC/PCM mixer cleanly); drag, scroll, or use arrow keys."));
    connect(sldSpk_, &QSlider::valueChanged, this, [this](int v) { if (!updatingUi_) onSpkSliderMoved(v); });
    connect(sldSpk_, &QSlider::sliderReleased, this, [this]() { sendSpkVolNow(); });
    spkRow->addWidget(sldSpk_, 1);
    btnSpkMute_ = new QPushButton(QStringLiteral("Mute Spk"), hwCard);
    btnSpkMute_->setToolTip(QStringLiteral("Mute Master Output — Silences physical speakers/headphones cleanly without disconnecting client audio streams."));
    connect(btnSpkMute_, &QPushButton::clicked, this, [this]() {
        const int nextMute = status_.desktop_muted ? 0 : 1;
        sendSpkVolNow(nextMute);
        status_.desktop_muted = nextMute;
        btnSpkMute_->setText(nextMute ? QStringLiteral("Unmute Spk") : QStringLiteral("Mute Spk"));
    });
    spkRow->addWidget(btnSpkMute_);
    hwCol->addLayout(spkRow);

    auto* micRow = new QHBoxLayout();
    lblMic_ = new QLabel(QStringLiteral("Mic Capture (100%):"), hwCard);
    lblMic_->setMinimumWidth(135);
    micRow->addWidget(lblMic_);
    sldMic_ = new StepVolumeSlider([this]() { return volStepPct_; }, Qt::Horizontal, hwCard);
    sldMic_->setRange(0, 150);
    sldMic_->setValue(100);
    sldMic_->setToolTip(QStringLiteral("Master Mic Capture Gain — Adjusts software microphone input gain (0–150%) delivered to recording and voice applications."));
    connect(sldMic_, &QSlider::valueChanged, this, [this](int v) { if (!updatingUi_) onMicSliderMoved(v); });
    connect(sldMic_, &QSlider::sliderReleased, this, [this]() { sendMicVolNow(); });
    micRow->addWidget(sldMic_, 1);
    btnMicMute_ = new QPushButton(QStringLiteral("Mute Mic"), hwCard);
    btnMicMute_->setToolTip(QStringLiteral("Mute Microphone — Silences captured microphone audio across all clients without closing the ALSA capture device."));
    connect(btnMicMute_, &QPushButton::clicked, this, [this]() {
        const int nextMute = status_.desktop_mic_muted ? 0 : 1;
        sendMicVolNow(nextMute);
        status_.desktop_mic_muted = nextMute;
        btnMicMute_->setText(nextMute ? QStringLiteral("Unmute Mic") : QStringLiteral("Mute Mic"));
    });
    micRow->addWidget(btnMicMute_);
    hwCol->addLayout(micRow);

    lblMidiStatus_ = new QLabel(QStringLiteral("DAW MIDI Hub: Auto-Detecting (0 key presses)"), hwCard);
    lblMidiStatus_->setWordWrap(true);
    lblMidiStatus_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Minimum);
    hwCol->addWidget(lblMidiStatus_);
    botLay->addWidget(hwCard, 5);

    auto* sessCard = new QFrame(tabStudio);
    sessCard->setObjectName(QStringLiteral("Card"));
    auto* sessCol = new QVBoxLayout(sessCard);
    sessCol->setContentsMargins(10, 6, 10, 6);
    sessCol->setSpacing(4);
    lblSessHdr_ = new QLabel(QStringLiteral("AUTO-RECOGNIZED FULL-DUPLEX SESSIONS (0)"), sessCard);
    lblSessHdr_->setObjectName(QStringLiteral("SectionHeader"));
    sessCol->addWidget(lblSessHdr_);

    tblSessions_ = new QTableWidget(0, 8, sessCard);
    tblSessions_->setHorizontalHeaderLabels({
        QStringLiteral("ID"), QStringLiteral("Dir"), QStringLiteral("Application"),
        QStringLiteral("Mode"), QStringLiteral("Format"), QStringLiteral("Volume"),
        QStringLiteral("Mute"), QStringLiteral("Route")});
    auto* shdr = tblSessions_->horizontalHeader();
    shdr->setMinimumSectionSize(28);
    shdr->setSectionResizeMode(0, QHeaderView::ResizeToContents);
    shdr->setSectionResizeMode(1, QHeaderView::ResizeToContents);
    shdr->setSectionResizeMode(2, QHeaderView::Stretch);
    shdr->setSectionResizeMode(3, QHeaderView::ResizeToContents);
    shdr->setSectionResizeMode(4, QHeaderView::ResizeToContents);
    shdr->setSectionResizeMode(5, QHeaderView::Fixed);
    shdr->setSectionResizeMode(6, QHeaderView::ResizeToContents);
    shdr->setSectionResizeMode(7, QHeaderView::Fixed);
    tblSessions_->setColumnWidth(5, scaledPx(96, fontPct_));
    tblSessions_->setColumnWidth(7, scaledPx(126, fontPct_));
    tblSessions_->setMinimumHeight(130);
    tblSessions_->verticalHeader()->setVisible(false);
    tblSessions_->verticalHeader()->setDefaultSectionSize(32);
    tblSessions_->setEditTriggers(QAbstractItemView::NoEditTriggers);
    tblSessions_->setSelectionMode(QAbstractItemView::NoSelection);
    sessCol->addWidget(tblSessions_);
    botLay->addWidget(sessCard, 7);

    studioLay->addLayout(botLay, 3);
    if (spectrum_) spectrum_->setMinimumHeight(140);
    scroll->setWidget(tabStudio);
    root->addWidget(scroll, 1);
}

void StudioController::buildTopologyWindow() {
    introWin_ = new PopToolWindow(
        QStringLiteral("Salsa — Auto-Graph Topology & Owner's Manual"),
        [this](QCloseEvent*) {
            if (closingAll_) return true;
            if (studioEntered_ || (mainWin_ && mainWin_->isVisible())) {
                introWin_->hide();
                syncGraphTimer();
                return false;
            }
            closingAll_ = true;
            QApplication::quit();
            return true;
        });
    QSize scrSz = QGuiApplication::primaryScreen() ? QGuiApplication::primaryScreen()->availableSize() : QSize(1600, 960);
    introWin_->resize(
        std::min(scrSz.width() - 24, scaledPx(1340, fontPct_)),
        std::min(scrSz.height() - 24, scaledPx(840, fontPct_)));

    auto* graphLayout = new QVBoxLayout(introWin_);
    graphLayout->setContentsMargins(10, 8, 10, 8);
    graphLayout->setSpacing(8);

    // Top Card: Live Signal Topology Header + Auto-Graph Canvas
    auto* graphCard = new QFrame(introWin_);
    graphCard->setObjectName(QStringLiteral("Card"));
    auto* graphBox = new QVBoxLayout(graphCard);
    graphBox->setContentsMargins(14, 10, 14, 10);
    graphBox->setSpacing(6);

    auto* graphHdrRow = new QHBoxLayout();
    auto* lblGTitle = new QLabel(
        QStringLiteral("AUTOMATIC AUDIO, MIDI & VIDEO SIGNAL TOPOLOGY (100% SELF-WIRING — CLICK ANY NODE TO INSPECT)"),
        graphCard);
    lblGTitle->setObjectName(QStringLiteral("SectionHeader"));
    graphHdrRow->addWidget(lblGTitle);
    graphHdrRow->addStretch(1);

    auto* lblAutoBadge = new QLabel(
        QStringLiteral("✔ %1 LIVE | MASTER CLOCK | HW ANALOG DAC | DYNAMIC EQ | BT ABR")
            .arg(SalsaStr(salsa_version_str()).toQString()),
        graphCard);
    lblAutoBadge->setObjectName(QStringLiteral("AutoBadgeLabel"));
    graphHdrRow->addWidget(lblAutoBadge);

    fontBarIntro_ = new FontScaleBar(fontPct_, true, graphCard);
    connect(fontBarIntro_, &FontScaleBar::fontScaleChanged, this, [this](int p) { setFontScalePctAndSync(p); });
    graphHdrRow->addWidget(fontBarIntro_);
    graphBox->addLayout(graphHdrRow);

    graphCanvas_ = new SalsaAutoGraphCanvas(graphCard);
    connect(graphCanvas_, &SalsaAutoGraphCanvas::nodeClicked, this, [this](const QString& key) {
        selectManualChapter(key);
    });
    graphBox->addWidget(graphCanvas_, 1);

    // Middle Card: Owner's Manual Chapter Tabs + Rich Text Viewer
    auto* manCard = new QFrame(introWin_);
    manCard->setObjectName(QStringLiteral("Card"));
    auto* manBox = new QVBoxLayout(manCard);
    manBox->setContentsMargins(14, 10, 14, 10);
    manBox->setSpacing(8);

    auto* chapBar = new QHBoxLayout();
    chapBar->setSpacing(4);
    auto* lblMan = new QLabel(QStringLiteral("OWNER'S MANUAL:"), manCard);
    lblMan->setObjectName(QStringLiteral("SectionHeader"));
    chapBar->addWidget(lblMan);

    const QJsonArray chArr = parseJsonArr(SalsaStr(salsa_manual_chapters_json()).toQString());
    for (const QJsonValue& cv : chArr) {
        const QJsonObject co = cv.toObject();
        const QString key = co.value(QStringLiteral("key")).toString();
        const QString lbl = co.value(QStringLiteral("label")).toString();
        auto* b = new QPushButton(lbl, manCard);
        b->setObjectName(QStringLiteral("CompactBtn"));
        b->setToolTip(QStringLiteral("Owner's Manual Chapter — Jumps the guide viewer below to documentation for '%1'.").arg(lbl));
        connect(b, &QPushButton::clicked, this, [this, key]() { selectManualChapter(key); });
        manualBtns_.insert(key, b);
        chapBar->addWidget(b);
    }

    auto* btnSync = new QPushButton(QStringLiteral("Sync Now"), manCard);
    btnSync->setObjectName(QStringLiteral("PrimaryBtn"));
    btnSync->setToolTip(QStringLiteral("Sync Audio Modules Now — Verifies host library ABIs and rebuilds ALSA/Pulse/JACK shims and config files if needed."));
    connect(btnSync, &QPushButton::clicked, this, [this]() {
        const SalsaStr rep(salsa_run_sync_modules(true));
        selectManualChapter(QStringLiteral("autosync"));
        showReportDialog(introWin_, QStringLiteral("Salsa Audio Module Auto-Sync Report"), rep.toQString(), currentTheme_, fontPct_, 760, 360);
    });
    chapBar->addWidget(btnSync);
    chapBar->addStretch(1);
    manBox->addLayout(chapBar);

    manualBrowser_ = new QTextBrowser(manCard);
    manualBrowser_->setObjectName(QStringLiteral("ManualBrowser"));
    manualBrowser_->setOpenExternalLinks(false);
    manBox->addWidget(manualBrowser_, 1);

    auto* splitter = new QSplitter(Qt::Vertical, introWin_);
    splitter->setChildrenCollapsible(false);
    splitter->addWidget(graphCard);
    splitter->addWidget(manCard);
    splitter->setStretchFactor(0, 13);
    splitter->setStretchFactor(1, 7);
    splitter->setSizes({500, 240});
    graphLayout->addWidget(splitter, 1);

    // Bottom Footer Card (exact 1:1 match to v6.9-beta)
    auto* introFooter = new QFrame(introWin_);
    introFooter->setObjectName(QStringLiteral("Card"));
    auto* footRow = new QHBoxLayout(introFooter);
    footRow->setContentsMargins(14, 8, 14, 8);
    footRow->setSpacing(12);

    chkHideIntroStartup_ = new QCheckBox(QStringLiteral("Don't show Topology Intro on startup"), introFooter);
    chkHideIntroStartup_->setObjectName(QStringLiteral("EqEnableCheck"));
    chkHideIntroStartup_->setChecked(!guiSettings_.value(QStringLiteral("show_topology_intro")).toBool(true));
    chkHideIntroStartup_->setToolTip(QStringLiteral("Skip Topology Intro on Launch — Opens directly into the Studio Mixer window next launch; press F1 anytime to reopen Topology."));
    connect(chkHideIntroStartup_, &QCheckBox::toggled, this, [this](bool hide) {
        guiSettings_.insert(QStringLiteral("show_topology_intro"), !hide);
        updateGuiSettingsPatch(QJsonObject{{QStringLiteral("show_topology_intro"), !hide}});
        if (chkShowIntroAdv_) {
            chkShowIntroAdv_->blockSignals(true);
            chkShowIntroAdv_->setChecked(!hide);
            chkShowIntroAdv_->blockSignals(false);
        }
    });
    footRow->addWidget(chkHideIntroStartup_);

    auto* lblHint = new QLabel(QStringLiteral("Shortcuts: Enter / Esc -> Studio • F1 -> Topology • Ctrl+, -> Advanced Options"), introFooter);
    lblHint->setObjectName(QStringLiteral("BandDynLabel"));
    footRow->addWidget(lblHint, 1);

    auto* btnAboutIntro = new QPushButton(QStringLiteral("About / LGPL-3.0"), introFooter);
    btnAboutIntro->setToolTip(QStringLiteral("About & LGPL-3.0 License — Opens Salsa copyright, warranty disclaimer, relinking rights, and COPYING/COPYING.LESSER viewer."));
    connect(btnAboutIntro, &QPushButton::clicked, this, [this]() {
        showAboutLicenseDialog(introWin_, QStringLiteral("Salsa Studio"), currentTheme_, fontPct_);
    });
    footRow->addWidget(btnAboutIntro);
    auto* btnAdv = new QPushButton(QStringLiteral("Advanced Options (Ctrl+,)"), introFooter);
    btnAdv->setToolTip(QStringLiteral("Advanced Options (Ctrl+,) — Opens parametric band table, crossfeed, hardware analog stages, shortcuts, and diagnostics."));
    connect(btnAdv, &QPushButton::clicked, this, [this]() { openAdvancedWindow(); });
    footRow->addWidget(btnAdv);

    auto* btnEnter = new QPushButton(QStringLiteral("Enter Studio Mixer  ->"), introFooter);
    btnEnter->setObjectName(QStringLiteral("PrimaryBtn"));
    btnEnter->setMinimumHeight(32);
    btnEnter->setToolTip(QStringLiteral("Enter Studio Mixer — Switches focus to the full 10-band Dynamic EQ, mixer, and active client session controls."));
    connect(btnEnter, &QPushButton::clicked, this, [this]() { enterStudioFromTopology(); });
    footRow->addWidget(btnEnter);
    graphLayout->addWidget(introFooter);

    selectManualChapter(QStringLiteral("overview"));
}

void StudioController::buildAdvancedWindow() {
    advWin_ = new PopToolWindow(
        QStringLiteral("Salsa — Advanced Options, DSP, Hardware Analog & Diagnostics"),
        [this](QCloseEvent*) {
            if (hwSendTimer_ && hwSendTimer_->isActive()) {
                hwSendTimer_->stop();
                onHwAnalogCfgChanged();
            }
            if (closingAll_) return true;
            if ((!mainWin_ || !mainWin_->isVisible()) && (!introWin_ || !introWin_->isVisible())) {
                closingAll_ = true;
                QApplication::quit();
                return true;
            }
            advWin_->hide();
            return false;
        });
    advWin_->resize(scaledPx(1240, fontPct_), scaledPx(740, fontPct_));

    auto* advRoot = new QVBoxLayout(advWin_);
    advRoot->setContentsMargins(12, 10, 12, 10);
    advRoot->setSpacing(8);

    auto* hdrCard = new QFrame(advWin_);
    hdrCard->setObjectName(QStringLiteral("Card"));
    auto* hcLay = new QHBoxLayout(hdrCard);
    hcLay->setContentsMargins(12, 8, 12, 8);
    auto* lblAh = new QLabel(QStringLiteral("SALSA — ADVANCED OPTIONS, DSP, HARDWARE ANALOG & DIAGNOSTICS"), hdrCard);
    lblAh->setObjectName(QStringLiteral("TitleBanner"));
    hcLay->addWidget(lblAh, 1);

    fontBarAdv_ = new FontScaleBar(fontPct_, false, hdrCard);
    connect(fontBarAdv_, &FontScaleBar::fontScaleChanged, this, [this](int p) { setFontScalePctAndSync(p); });
    hcLay->addWidget(fontBarAdv_);

    auto* btnAboutAdv = new QPushButton(QStringLiteral("About / LGPL-3.0"), hdrCard);
    btnAboutAdv->setToolTip(QStringLiteral("About & LGPL-3.0 License — Opens Salsa copyright, no-warranty notice, third-party dlopen disclosures, and COPYING/COPYING.LESSER texts."));
    connect(btnAboutAdv, &QPushButton::clicked, this, [this]() {
        showAboutLicenseDialog(advWin_, QStringLiteral("Salsa Studio"), currentTheme_, fontPct_);
    });
    hcLay->addWidget(btnAboutAdv);
    auto* btnCloseAdv = new QPushButton(QStringLiteral("Close (Esc)"), hdrCard);
    btnCloseAdv->setToolTip(QStringLiteral("Close Advanced Options (Esc) — Hides this window while keeping all DSP, hardware, and shortcut settings active."));
    connect(btnCloseAdv, &QPushButton::clicked, advWin_, &QWidget::close);
    hcLay->addWidget(btnCloseAdv);
    advRoot->addWidget(hdrCard);

    auto* scroll = new QScrollArea(advWin_);
    scroll->setWidgetResizable(true);
    scroll->setFrameShape(QFrame::NoFrame);
    auto* body = new QWidget(scroll);
    auto* bodyLay = new QVBoxLayout(body);
    bodyLay->setContentsMargins(0, 0, 0, 0);
    bodyLay->setSpacing(8);

    auto* colsRow = new QHBoxLayout();
    colsRow->setSpacing(10);
    auto* leftCol = new QVBoxLayout();
    auto* rightCol = new QVBoxLayout();

    // L1: Diagnostics, Debugger & Audio Services
    auto* diagCard = new QFrame(body);
    diagCard->setObjectName(QStringLiteral("Card"));
    auto* dcLay = new QVBoxLayout(diagCard);
    dcLay->setContentsMargins(12, 8, 12, 8);
    dcLay->setSpacing(6);
    auto* lblDiagH = new QLabel(QStringLiteral("DIAGNOSTICS, STANDALONE DEBUGGER & AUDIO SERVICES"), diagCard);
    lblDiagH->setObjectName(QStringLiteral("SectionHeader"));
    dcLay->addWidget(lblDiagH);

    auto* gDiag = new QGridLayout();
    gDiag->setSpacing(6);
    auto* btnOpenDbg = new QPushButton(QStringLiteral("Open Live Debugger Window (Ctrl+D)"), diagCard);
    btnOpenDbg->setObjectName(QStringLiteral("PrimaryBtn"));
    btnOpenDbg->setToolTip(QStringLiteral("Live Architecture Debugger (Ctrl+D) — Launches real-time event tracing, process leak audits, RT timing, and ScreenCast previews."));
    connect(btnOpenDbg, &QPushButton::clicked, this, [this]() { openLiveDebuggerWindow(); });
    gDiag->addWidget(btnOpenDbg, 0, 0, 1, 2);

    auto* btnBench = new QPushButton(QStringLiteral("Run Latency Benchmark"), diagCard);
    btnBench->setToolTip(QStringLiteral("Latency Benchmark — Runs an isolated virtual timing & loopback benchmark comparing Native WASAPI, Pulse, and ALSA shims without altering your live volume or interrupting playback."));
    connect(btnBench, &QPushButton::clicked, this, [this, btnBench]() {
        if (!btnBench->isEnabled()) return;
        btnBench->setEnabled(false);
        btnBench->setText(QStringLiteral("Running Benchmark..."));
        const QPointer<StudioController> pSelf(this);
        const QPointer<QPushButton> pBtn(btnBench);
        const QPointer<QWidget> pAdv(advWin_);
        const SalsaThemeData themeSnap = currentTheme_;
        const int fontSnap = fontPct_;
        QThread* thr = QThread::create([pSelf, pBtn, pAdv, themeSnap, fontSnap]() {
            const SalsaStr rep(salsa_run_latency_bench());
            const QString repStr = rep.toQString();
            QCoreApplication* app = QApplication::instance();
            if (!app) return;
            QMetaObject::invokeMethod(app, [pSelf, pBtn, pAdv, repStr, themeSnap, fontSnap]() {
                if (!pSelf) return;
                if (pBtn) {
                    pBtn->setEnabled(true);
                    pBtn->setText(QStringLiteral("Run Latency Benchmark"));
                }
                showReportDialog(pAdv ? pAdv.data() : nullptr, QStringLiteral("Salsa Latency Benchmark"), repStr, themeSnap, fontSnap);
            }, Qt::QueuedConnection);
        });
        connect(thr, &QThread::finished, thr, &QObject::deleteLater);
        thr->start();
    });
    gDiag->addWidget(btnBench, 1, 0);

    auto* btnAudit = new QPushButton(QStringLiteral("Run System Health Audit"), diagCard);
    btnAudit->setToolTip(QStringLiteral("System Health Audit — Checks sockets, ELF hardening, systemd services, BlueZ power, and build artifact freshness."));
    connect(btnAudit, &QPushButton::clicked, this, [this]() {
        const QJsonObject rep = parseJsonObj(SalsaStr(salsa_run_health_audit_json()).toQString());
        const QString pretty = QString::fromUtf8(QJsonDocument(rep).toJson(QJsonDocument::Indented));
        showReportDialog(advWin_, QStringLiteral("Salsa System Health & Process Audit"), pretty, currentTheme_, fontPct_);
    });
    gDiag->addWidget(btnAudit, 1, 1);

    auto* btnSync = new QPushButton(QStringLiteral("Sync Audio Modules Now"), diagCard);
    btnSync->setToolTip(QStringLiteral("Sync Audio Modules Now — Verifies host `.so` ABI symbol tables and rebuilds ALSA/Pulse/JACK shims via lwasapi-autosync."));
    connect(btnSync, &QPushButton::clicked, this, [this]() {
        const SalsaStr rep(salsa_run_sync_modules(true));
        showReportDialog(advWin_, QStringLiteral("Salsa Module Auto-Sync Report"), rep.toQString(), currentTheme_, fontPct_, 760, 360);
    });
    gDiag->addWidget(btnSync, 2, 0);

    auto* btnMidi = new QPushButton(QStringLiteral("Send Test MIDI (C4)"), diagCard);
    btnMidi->setToolTip(QStringLiteral("Send Test MIDI (C4) — Injects a synthetic Note-On C4 packet into the LWASAPI MIDI Hub to test synth or debugger reception."));
    connect(btnMidi, &QPushButton::clicked, this, []() { salsa_send_test_midi(60, 115); });
    gDiag->addWidget(btnMidi, 2, 1);

    btnRstEngine_ = new QPushButton(QStringLiteral("Restart Audio Engine"), diagCard);
    btnRstEngine_->setToolTip(QStringLiteral("Restart Audio Engine — Restarts `lwasapi-audiodg.service` and re-establishes hardware, BlueZ, and Pulse clients."));
    connect(btnRstEngine_, &QPushButton::clicked, this, [this]() { runSystemdRestart(QStringLiteral("lwasapi-audiodg.service"), btnRstEngine_); });
    gDiag->addWidget(btnRstEngine_, 3, 0);

    btnRstTray_ = new QPushButton(QStringLiteral("Restart Tray Service"), diagCard);
    btnRstTray_->setToolTip(QStringLiteral("Restart Tray Service — Restarts `lwasapi-tray.service` to reload the taskbar Quick Mixer indicator."));
    connect(btnRstTray_, &QPushButton::clicked, this, [this]() { runSystemdRestart(QStringLiteral("lwasapi-tray.service"), btnRstTray_); });
    gDiag->addWidget(btnRstTray_, 3, 1);
    dcLay->addLayout(gDiag);

    auto* chipRow = new QHBoxLayout();
    lblChipSteam_ = new QLabel(QStringLiteral("Steam: ..."), diagCard);
    lblChipLaunchers_ = new QLabel(QStringLiteral("Launchers: ..."), diagCard);
    lblChipPacman_ = new QLabel(QStringLiteral("Pacman Hook: ..."), diagCard);
    for (QLabel* ch : {lblChipSteam_, lblChipLaunchers_, lblChipPacman_}) {
        ch->setObjectName(QStringLiteral("SavedChipLabel"));
        chipRow->addWidget(ch);
    }
    chipRow->addStretch(1);
    dcLay->addLayout(chipRow);

    auto* hookRow = new QHBoxLayout();
    btnTogglePacman_ = new QPushButton(QStringLiteral("Toggle Pacman Hook (pkexec)"), diagCard);
    btnTogglePacman_->setObjectName(QStringLiteral("CompactBtn"));
    btnTogglePacman_->setCheckable(true);
    btnTogglePacman_->setToolTip(QStringLiteral("Pacman Autosync Hook — Installs or removes `/etc/pacman.d/hooks/99-salsa-autosync.hook` via pkexec so package upgrades auto-resync shims."));
    connect(btnTogglePacman_, &QPushButton::clicked, this, [this]() {
        const SalsaStr msg(salsa_toggle_pacman_hook());
        refreshAdvIntegration();
        if (!msg.empty()) QMessageBox::information(advWin_, QStringLiteral("Pacman Hook"), msg.toQString());
    });
    hookRow->addWidget(btnTogglePacman_, 1);

    chkAutosyncNow_ = new QCheckBox(QStringLiteral("Auto-Sync Watcher + Sync Now"), diagCard);
    chkAutosyncNow_->setToolTip(QStringLiteral("Background ABI Watcher — Enables or disables `lwasapi-module-watcher.path/.timer` so library updates trigger automatic shim rebuilds."));
    connect(chkAutosyncNow_, &QCheckBox::clicked, this, [this](bool checked) {
        const SalsaStr msg(salsa_toggle_autosync_watcher(checked));
        refreshAdvIntegration();
        Q_UNUSED(msg);
    });
    hookRow->addWidget(chkAutosyncNow_);
    dcLay->addLayout(hookRow);
    leftCol->addWidget(diagCard);

    // L2: Engine Ownership & Appearance
    auto* engCard = new QFrame(body);
    engCard->setObjectName(QStringLiteral("Card"));
    auto* ecLay = new QVBoxLayout(engCard);
    ecLay->setContentsMargins(12, 8, 12, 8);
    ecLay->setSpacing(6);
    auto* lblEngH = new QLabel(QStringLiteral("ENGINE OWNERSHIP, STARTUP & APPEARANCE"), engCard);
    lblEngH->setObjectName(QStringLiteral("SectionHeader"));
    ecLay->addWidget(lblEngH);

    btnEngineToggle_ = new QPushButton(QStringLiteral("Engine: SALSA NATIVE AUDIO (Revert to PW/WP)"), engCard);
    btnEngineToggle_->setObjectName(QStringLiteral("PrimaryBtn"));
    btnEngineToggle_->setToolTip(QStringLiteral("Switch Audio Engine Ownership — Toggles between Pure Salsa Native Audio (Video-Only PipeWire broker) and upstream PipeWire/WirePlumber."));
    connect(btnEngineToggle_, &QPushButton::clicked, this, [this]() {
        const QJsonObject st = parseJsonObj(SalsaStr(salsa_get_integration_status_json()).toQString());
        const bool active = st.value(QStringLiteral("engine_active")).toBool(true);
        const SalsaStr out(salsa_switch_engine_mode(active));
        refreshAdvIntegration();
        QMessageBox::information(advWin_, QStringLiteral("Audio Engine Ownership"), out.toQString());
    });
    ecLay->addWidget(btnEngineToggle_);

    chkShowIntroAdv_ = new QCheckBox(QStringLiteral("Show Auto-Graph Topology Intro window on startup"), engCard);
    chkShowIntroAdv_->setChecked(guiSettings_.value(QStringLiteral("show_topology_intro")).toBool(true));
    chkShowIntroAdv_->setToolTip(QStringLiteral("Startup Topology Intro — Shows the Auto-Graph Topology & Owner's Manual window first whenever launching Salsa Studio."));
    connect(chkShowIntroAdv_, &QCheckBox::toggled, this, [this](bool on) {
        guiSettings_.insert(QStringLiteral("show_topology_intro"), on);
        updateGuiSettingsPatch(QJsonObject{{QStringLiteral("show_topology_intro"), on}});
        if (chkHideIntroStartup_) {
            chkHideIntroStartup_->blockSignals(true);
            chkHideIntroStartup_->setChecked(!on);
            chkHideIntroStartup_->blockSignals(false);
        }
    });
    ecLay->addWidget(chkShowIntroAdv_);

    auto* rowApp = new QHBoxLayout();
    auto* btnThemeEd = new QPushButton(QStringLiteral("Open Custom Theme Studio..."), engCard);
    btnThemeEd->setToolTip(QStringLiteral("Custom Theme Studio — Opens an interactive color palette editor with Auto-Shade harmony and a live miniature Salsa UI preview."));
    connect(btnThemeEd, &QPushButton::clicked, this, [this]() { openCustomThemeDialog(); });
    rowApp->addWidget(btnThemeEd, 1);

    auto* btnResetSafe = new QPushButton(QStringLiteral("Reset Settings to Safe Defaults"), engCard);
    btnResetSafe->setObjectName(QStringLiteral("CompactBtn"));
    btnResetSafe->setToolTip(QStringLiteral("Reset Settings to Safe Defaults — Flattens EQ, reverts discrete hardware analog stages to Auto-Preserve, and restores the Topology intro."));
    connect(btnResetSafe, &QPushButton::clicked, this, [this]() {
        onResetFlatClicked();
        onHwAnalogReset();
        chkShowIntroAdv_->setChecked(true);
    });
    rowApp->addWidget(btnResetSafe);
    ecLay->addLayout(rowApp);

    auto* rowCfg = new QHBoxLayout();
    btnSaveCfgAdv_ = new QPushButton(QStringLiteral("Save Settings"), engCard);
    btnSaveCfgAdv_->setObjectName(QStringLiteral("PrimaryBtn"));
    btnSaveCfgAdv_->setToolTip(QStringLiteral("Save & Overwrite Settings (Ctrl+S) — Validates and overwrites ~/.config/lwasapi/salsa_settings.json with all current engine, BT, analog, EQ, volume, and GUI settings."));
    connect(btnSaveCfgAdv_, &QPushButton::clicked, this, [this]() { onSaveOverwriteConfigClicked(); });
    rowCfg->addWidget(btnSaveCfgAdv_, 1);

    btnReloadCfg_ = new QPushButton(QStringLiteral("Reload Saved Config"), engCard);
    btnReloadCfg_->setObjectName(QStringLiteral("CompactBtn"));
    btnReloadCfg_->setToolTip(QStringLiteral("Reload Saved Config — Re-reads ~/.config/lwasapi/salsa_settings.json and reapplies saved settings to the daemon and Studio UI."));
    connect(btnReloadCfg_, &QPushButton::clicked, this, [this]() { onReloadConfigClicked(); });
    rowCfg->addWidget(btnReloadCfg_);

    btnOpenCfgAdv_ = new QPushButton(QStringLiteral("Open Config File"), engCard);
    btnOpenCfgAdv_->setObjectName(QStringLiteral("CompactBtn"));
    btnOpenCfgAdv_->setToolTip(QStringLiteral("Open Config File — Opens ~/.config/lwasapi/salsa_settings.json in your system editor; click Reload Saved Config after making manual edits."));
    connect(btnOpenCfgAdv_, &QPushButton::clicked, this, [this]() { onOpenConfigFileClicked(); });
    rowCfg->addWidget(btnOpenCfgAdv_);
    ecLay->addLayout(rowCfg);

    lblCfgSaveStatus_ = new QLabel(
        QStringLiteral("Config: %1").arg(SalsaStr(salsa_settings_path_str()).toQString()), engCard);
    lblCfgSaveStatus_->setObjectName(QStringLiteral("DiagStatusLabel"));
    lblCfgSaveStatus_->setWordWrap(true);
    lblCfgSaveStatus_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Minimum);
    ecLay->addWidget(lblCfgSaveStatus_);

    leftCol->addWidget(engCard);

    // R1: Advanced EQ, Crossfeed & Per-Band Parametric Table
    auto* eqAdvCard = new QFrame(body);
    eqAdvCard->setObjectName(QStringLiteral("Card"));
    auto* qaLay = new QVBoxLayout(eqAdvCard);
    qaLay->setContentsMargins(12, 8, 12, 8);
    qaLay->setSpacing(6);
    auto* lblQa = new QLabel(QStringLiteral("ADVANCED EQ, AUTOEQ & SPATIAL DSP"), eqAdvCard);
    lblQa->setObjectName(QStringLiteral("SectionHeader"));
    qaLay->addWidget(lblQa);

    auto* rQa1 = new QHBoxLayout();
    chkLimiter_ = new QCheckBox(QStringLiteral("Soft-Knee Mastering Limiter"), eqAdvCard);
    chkLimiter_->setChecked(true);
    chkLimiter_->setToolTip(QStringLiteral("Soft-Knee Mastering Limiter — Gently compresses peaks near 0 dBFS so hot mixes and positive EQ boosts cannot hard-clip."));
    connect(chkLimiter_, &QCheckBox::toggled, this, [this]() { if (!updatingUi_) pushEqToEngine(); });
    rQa1->addWidget(chkLimiter_);

    chkEqPerDev_ = new QCheckBox(QStringLiteral("Per-Device EQ"), eqAdvCard);
    chkEqPerDev_->setChecked(guiSettings_.value(QStringLiteral("eq_per_device")).toBool(false));
    chkEqPerDev_->setToolTip(QStringLiteral("Per-Device EQ Memory — Saves and restores separate EQ profiles automatically whenever the active output endpoint changes."));
    connect(chkEqPerDev_, &QCheckBox::toggled, this, [this](bool on) { onPerDeviceEqToggled(on); });
    rQa1->addWidget(chkEqPerDev_);
    rQa1->addStretch(1);
    qaLay->addLayout(rQa1);

    auto* rQa2 = new QHBoxLayout();
    auto* btnImp = new QPushButton(QStringLiteral("Import AutoEQ .txt"), eqAdvCard);
    btnImp->setObjectName(QStringLiteral("CompactBtn"));
    btnImp->setToolTip(QStringLiteral("Import AutoEQ .txt — Parses an EqualizerAPO / Oratory1990 text profile into Salsa's preamp and 10 parametric filter slots."));
    connect(btnImp, &QPushButton::clicked, this, [this]() { onImportAutoEqClicked(); });
    rQa2->addWidget(btnImp);

    auto* btnRel = new QPushButton(QStringLiteral("Reload AutoEQ .txt"), eqAdvCard);
    btnRel->setObjectName(QStringLiteral("CompactBtn"));
    btnRel->setToolTip(QStringLiteral("Reload AutoEQ .txt — Re-reads the currently bound AutoEQ file from disk and applies any updates."));
    connect(btnRel, &QPushButton::clicked, this, [this]() { onReloadAutoEqClicked(); });
    rQa2->addWidget(btnRel);
    rQa2->addStretch(1);
    qaLay->addLayout(rQa2);

    auto* rXf = new QHBoxLayout();
    chkCrossfeed_ = new QCheckBox(QStringLiteral("Bauer Crossfeed"), eqAdvCard);
    chkCrossfeed_->setToolTip(QStringLiteral("Bauer Stereo Crossfeed — Blends frequency-shaped interaural delay between left and right ears for natural speaker-like headphone imaging."));
    connect(chkCrossfeed_, &QCheckBox::toggled, this, [this]() {
        if (!updatingUi_) pushEqToEngine();
    });
    rXf->addWidget(chkCrossfeed_);

    sldCrossfeed_ = new QSlider(Qt::Horizontal, eqAdvCard);
    sldCrossfeed_->setRange(0, 100);
    sldCrossfeed_->setValue(30);
    sldCrossfeed_->setToolTip(QStringLiteral("Crossfeed Blend Level — Sets opposite-channel feed mix (0–100%) when Bauer Crossfeed is enabled."));
    connect(sldCrossfeed_, &QSlider::valueChanged, this, [this](int v) {
        lblCrossfeed_->setText(formatPctText(v));
        if (!updatingUi_) pushEqToEngine();
    });
    rXf->addWidget(sldCrossfeed_, 1);
    lblCrossfeed_ = new QLabel(formatPctText(30), eqAdvCard);
    lblCrossfeed_->setObjectName(QStringLiteral("ValMonoLabel"));
    lblCrossfeed_->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
    lblCrossfeed_->setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Preferred);
    rXf->addWidget(lblCrossfeed_);

    cmbCrossfeedCut_ = new QComboBox(eqAdvCard);
    cmbCrossfeedCut_->addItem(QStringLiteral("650 Hz"), 650);
    cmbCrossfeedCut_->addItem(QStringLiteral("700 Hz"), 700);
    cmbCrossfeedCut_->addItem(QStringLiteral("850 Hz"), 850);
    cmbCrossfeedCut_->setCurrentIndex(1);
    cmbCrossfeedCut_->setToolTip(QStringLiteral("Crossfeed Corner Frequency — Bessel low-pass cutoff (650/700/850 Hz) shaping how low/mid frequencies cross between ears."));
    connect(cmbCrossfeedCut_, &QComboBox::currentIndexChanged, this, [this]() { if (!updatingUi_) pushEqToEngine(); });
    rXf->addWidget(cmbCrossfeedCut_);
    qaLay->addLayout(rXf);

    tblParamBands_ = new QTableWidget(SALSA_EQ_BANDS, 4, eqAdvCard);
    tblParamBands_->setHorizontalHeaderLabels({QStringLiteral("Band"), QStringLiteral("Freq (Hz)"), QStringLiteral("Q"), QStringLiteral("Shape")});
    tblParamBands_->horizontalHeader()->setSectionResizeMode(QHeaderView::Stretch);
    tblParamBands_->verticalHeader()->setVisible(false);
    tblParamBands_->verticalHeader()->setDefaultSectionSize(24);
    tblParamBands_->setMinimumHeight(145);
    tblParamBands_->setMaximumHeight(190);
    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
        auto* itIdx = new QTableWidgetItem(QStringLiteral("B%1").arg(i + 1));
        itIdx->setFlags(itIdx->flags() & ~Qt::ItemIsEditable);
        tblParamBands_->setItem(i, 0, itIdx);

        auto* spnHz = new QDoubleSpinBox(tblParamBands_);
        spnHz->setRange(15.0, 22000.0);
        spnHz->setDecimals(1);
        spnHz->setValue(DEFAULT_BAND_FREQS_HZ[i]);
        spnHz->setToolTip(QStringLiteral("Band %1 Frequency (Hz) — Sets this biquad filter's center or shelf corner frequency (15–22000 Hz) live.").arg(i + 1));
        connect(spnHz, &QDoubleSpinBox::valueChanged, this, [this, i](double val) {
            if (updatingUi_) return;
            paramExt_.band_freq_hz[i] = static_cast<float>(val);
            bandFreqLbls_[i]->setText(fmtFreqHz(paramExt_.band_freq_hz[i]));
            pushEqToEngine();
        });
        tblParamBands_->setCellWidget(i, 1, spnHz);

        auto* spnQ = new QDoubleSpinBox(tblParamBands_);
        spnQ->setRange(0.20, 10.00);
        spnQ->setSingleStep(0.10);
        spnQ->setDecimals(2);
        spnQ->setValue(1.414);
        spnQ->setToolTip(QStringLiteral("Band %1 Q Factor — Adjusts filter bandwidth from wide shelf/bell (0.20) to narrow surgical notch (10.00).").arg(i + 1));
        connect(spnQ, &QDoubleSpinBox::valueChanged, this, [this, i](double val) {
            if (updatingUi_) return;
            eqCfg_.band_q[i] = static_cast<float>(val);
            pushEqToEngine();
        });
        tblParamBands_->setCellWidget(i, 2, spnQ);

        auto* cmbType = new QComboBox(tblParamBands_);
        cmbType->addItem(QStringLiteral("Peak"), SALSA_FILTER_PEAK);
        cmbType->addItem(QStringLiteral("Low-Shelf"), SALSA_FILTER_LOW_SHELF);
        cmbType->addItem(QStringLiteral("High-Shelf"), SALSA_FILTER_HIGH_SHELF);
        cmbType->setToolTip(QStringLiteral("Band %1 Filter Shape — Selects Peak bell, Low-Shelf bass boost/cut, or High-Shelf treble tilt for this band.").arg(i + 1));
        connect(cmbType, &QComboBox::currentIndexChanged, this, [this, i, cmbType]() {
            if (updatingUi_) return;
            paramExt_.band_type[i] = static_cast<uint32_t>(cmbType->currentData().toInt());
            pushEqToEngine();
        });
        tblParamBands_->setCellWidget(i, 3, cmbType);
    }
    qaLay->addWidget(tblParamBands_);
    rightCol->addWidget(eqAdvCard);

    // R2: Hardware Analog Stage
    auto* hwaCard = new QFrame(body);
    hwaCard->setObjectName(QStringLiteral("Card"));
    auto* haLay = new QVBoxLayout(hwaCard);
    haLay->setContentsMargins(12, 8, 12, 8);
    haLay->setSpacing(6);
    auto* lblHa = new QLabel(QStringLiteral("HARDWARE ANALOG STAGE (STRIX / DISCRETE SOUND CARDS)"), hwaCard);
    lblHa->setObjectName(QStringLiteral("SectionHeader"));
    haLay->addWidget(lblHa);

    auto* haR1 = new QHBoxLayout();
    chkHwDacUnity_ = new QCheckBox(QStringLiteral("HW DAC Unity"), hwaCard);
    chkHwDacUnity_->setChecked(true);
    chkHwDacUnity_->setToolTip(QStringLiteral("Hardware DAC Unity Gain — Pins supported soundcard DAC ALSA mixers to 0 dB unity so volume scaling stays pure 32-bit float in Salsa."));
    connect(chkHwDacUnity_, &QCheckBox::toggled, this, [this]() { if (!updatingUi_) onHwAnalogCfgChanged(); });
    haR1->addWidget(chkHwDacUnity_);

    cmbHwAmp_ = new QComboBox(hwaCard);
    cmbHwAmp_->addItem(QStringLiteral("Amp: Auto (Preserve)"), SALSA_HW_AMP_AUTO);
    cmbHwAmp_->addItem(QStringLiteral("Amp: Low (<=32Ω IEM)"), SALSA_HW_AMP_LOW);
    cmbHwAmp_->addItem(QStringLiteral("Amp: Med (64-150Ω)"), SALSA_HW_AMP_MED);
    cmbHwAmp_->addItem(QStringLiteral("Amp: High (600Ω TPA6120)"), SALSA_HW_AMP_HIGH);
    cmbHwAmp_->setToolTip(QStringLiteral("Discrete Headphone Amp Tier — Configures hardware impedance gain on supported audiophile cards (or preserves card defaults in Auto)."));
    connect(cmbHwAmp_, &QComboBox::currentIndexChanged, this, [this]() { if (!updatingUi_) onHwAnalogCfgChanged(); });
    haR1->addWidget(cmbHwAmp_);

    cmbHwRoute_ = new QComboBox(hwaCard);
    cmbHwRoute_->addItem(QStringLiteral("Route: Auto"), SALSA_HW_ROUTE_AUTO);
    cmbHwRoute_->addItem(QStringLiteral("Route: Headphones"), SALSA_HW_ROUTE_HP);
    cmbHwRoute_->addItem(QStringLiteral("Route: Line / Spk"), SALSA_HW_ROUTE_SPK);
    cmbHwRoute_->setToolTip(QStringLiteral("Hardware Analog Route — Switches discrete output relays between Headphones and Line/Speakers when exposed by the ALSA mixer."));
    connect(cmbHwRoute_, &QComboBox::currentIndexChanged, this, [this]() { if (!updatingUi_) onHwAnalogCfgChanged(); });
    haR1->addWidget(cmbHwRoute_);

    chkHwRails_ = new QCheckBox(QStringLiteral("0dB Rails"), hwaCard);
    chkHwRails_->setToolTip(QStringLiteral("0 dB Analog Master Rails — Pins supported discrete card analog output rails cleanly at full 0 dB reference headroom."));
    connect(chkHwRails_, &QCheckBox::toggled, this, [this]() { if (!updatingUi_) onHwAnalogCfgChanged(); });
    haR1->addWidget(chkHwRails_);
    haLay->addLayout(haR1);

    auto* haR2 = new QHBoxLayout();
    haR2->addWidget(new QLabel(QStringLiteral("Analog Drive:"), hwaCard));
    sldHwBoost_ = new QSlider(Qt::Horizontal, hwaCard);
    sldHwBoost_->setRange(0, 24);
    sldHwBoost_->setSingleStep(1);
    sldHwBoost_->setPageStep(2);
    sldHwBoost_->setToolTip(QStringLiteral("Analog / Pre-Limiter Drive — Applies +0.0 to +12.0 dB extra analog output drive (safely soft-clipped if Limiter is active)."));
    connect(sldHwBoost_, &QSlider::valueChanged, this, [this](int v) {
        lblHwBoost_->setText(formatBoostDbText(v * 0.5));
        if (!updatingUi_) scheduleHwAnalogSend(false);
    });
    connect(sldHwBoost_, &QSlider::sliderReleased, this, [this]() { onHwAnalogCfgChanged(); });
    haR2->addWidget(sldHwBoost_, 1);
    lblHwBoost_ = new QLabel(formatBoostDbText(0.0), hwaCard);
    lblHwBoost_->setObjectName(QStringLiteral("ValMonoLabel"));
    lblHwBoost_->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
    lblHwBoost_->setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Preferred);
    haR2->addWidget(lblHwBoost_);
    haLay->addLayout(haR2);

    auto* haR3 = new QHBoxLayout();
    haR3->addWidget(new QLabel(QStringLiteral("Mic HW Boost:"), hwaCard));
    sldHwMicBoost_ = new QSlider(Qt::Horizontal, hwaCard);
    sldHwMicBoost_->setRange(0, 24);
    sldHwMicBoost_->setSingleStep(1);
    sldHwMicBoost_->setPageStep(2);
    sldHwMicBoost_->setToolTip(QStringLiteral("Hardware Mic Preamp Boost — Configures supported ALSA capture boost (+0 to +24 dB) for quiet microphones."));
    connect(sldHwMicBoost_, &QSlider::valueChanged, this, [this](int v) {
        lblHwMicBoost_->setText(formatBoostDbText(static_cast<double>(v)));
        if (!updatingUi_) scheduleHwAnalogSend(false);
    });
    connect(sldHwMicBoost_, &QSlider::sliderReleased, this, [this]() { onHwAnalogCfgChanged(); });
    haR3->addWidget(sldHwMicBoost_, 1);
    lblHwMicBoost_ = new QLabel(formatBoostDbText(0.0), hwaCard);
    lblHwMicBoost_->setObjectName(QStringLiteral("ValMonoLabel"));
    lblHwMicBoost_->setAlignment(Qt::AlignRight | Qt::AlignVCenter);
    lblHwMicBoost_->setSizePolicy(QSizePolicy::Fixed, QSizePolicy::Preferred);
    haR3->addWidget(lblHwMicBoost_);

    auto* btnSafeHw = new QPushButton(QStringLiteral("Safe Defaults"), hwaCard);
    btnSafeHw->setToolTip(QStringLiteral("Safe Hardware Defaults — Reverts all analog amp, route, rail, and boost settings to Auto-Preserve."));
    connect(btnSafeHw, &QPushButton::clicked, this, [this]() { onHwAnalogReset(); });
    haR3->addWidget(btnSafeHw);

    btnSaveHw_ = new QPushButton(QStringLiteral("Save Settings"), hwaCard);
    btnSaveHw_->setObjectName(QStringLiteral("PrimaryBtn"));
    btnSaveHw_->setToolTip(QStringLiteral("Save & Overwrite Settings (Ctrl+S) — Pushes Hardware Analog Stage and all Studio settings directly into ~/.config/lwasapi/salsa_settings.json."));
    connect(btnSaveHw_, &QPushButton::clicked, this, [this]() { onSaveOverwriteConfigClicked(); });
    haR3->addWidget(btnSaveHw_);
    haLay->addLayout(haR3);

    lblHwAnalogDiag_ = new QLabel(QStringLiteral("HW Stage: Probing..."), hwaCard);
    lblHwAnalogDiag_->setObjectName(QStringLiteral("DiagStatusLabel"));
    lblHwAnalogDiag_->setWordWrap(true);
    lblHwAnalogDiag_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Minimum);
    lblHwAnalogDiag_->setMinimumWidth(0);
    haLay->addWidget(lblHwAnalogDiag_);
    rightCol->addWidget(hwaCard);

    leftCol->addStretch(1);
    rightCol->addStretch(1);
    colsRow->addLayout(leftCol, 1);
    colsRow->addLayout(rightCol, 1);
    bodyLay->addLayout(colsRow);

    // Bottom: Customizable Keyboard Shortcuts
    auto* scCard = new QFrame(body);
    scCard->setObjectName(QStringLiteral("Card"));
    auto* scLay = new QVBoxLayout(scCard);
    scLay->setContentsMargins(12, 8, 12, 8);
    scLay->setSpacing(6);

    auto* scHdr = new QHBoxLayout();
    auto* lblSc = new QLabel(QStringLiteral("CUSTOMIZABLE KEYBOARD SHORTCUTS"), scCard);
    lblSc->setObjectName(QStringLiteral("SectionHeader"));
    scHdr->addWidget(lblSc);
    scHdr->addStretch(1);
    auto* btnScReset = new QPushButton(QStringLiteral("Restore Defaults"), scCard);
    btnScReset->setObjectName(QStringLiteral("CompactBtn"));
    btnScReset->setToolTip(QStringLiteral("Restore Default Shortcuts — Reverts all Salsa Studio keyboard shortcuts to their built-in factory bindings."));
    connect(btnScReset, &QPushButton::clicked, this, [this]() {
        const QJsonArray defSc = parseJsonArr(SalsaStr(salsa_default_shortcuts_json()).toQString());
        QJsonObject scObj;
        for (const QJsonValue& v : defSc) {
            const QJsonObject o = v.toObject();
            const QString k = o.value(QStringLiteral("key")).toString();
            const QString seq = o.value(QStringLiteral("default_seq")).toString();
            shortcutsMap_.insert(k, seq);
            scObj.insert(k, seq);
            if (auto* ed = shortcutEdits_.value(k, nullptr)) ed->setText(seq);
        }
        updateGuiSettingsPatch(QJsonObject{{QStringLiteral("shortcuts"), scObj}});
        applyDynamicShortcuts();
    });
    scHdr->addWidget(btnScReset);
    scLay->addLayout(scHdr);

    auto* scGrid = new QGridLayout();
    scGrid->setHorizontalSpacing(14);
    scGrid->setVerticalSpacing(4);
    const QJsonArray defSc = parseJsonArr(SalsaStr(salsa_default_shortcuts_json()).toQString());
    int sIdx = 0;
    for (const QJsonValue& v : defSc) {
        const QJsonObject o = v.toObject();
        const QString k = o.value(QStringLiteral("key")).toString();
        const QString lbl = o.value(QStringLiteral("label")).toString();
        const QString cur = shortcutsMap_.value(k);
        const int r = sIdx / 3;
        const int c = (sIdx % 3) * 2;
        scGrid->addWidget(new QLabel(lbl + QStringLiteral(":"), scCard), r, c);
        auto* ed = new ShortcutCaptureEdit(k, cur, scCard);
        connect(ed, &ShortcutCaptureEdit::shortcutCaptured, this, [this](const QString& key, const QString& seq) {
            shortcutsMap_.insert(key, seq);
            QJsonObject scObj;
            for (auto it = shortcutsMap_.constBegin(); it != shortcutsMap_.constEnd(); ++it) scObj.insert(it.key(), it.value());
            updateGuiSettingsPatch(QJsonObject{{QStringLiteral("shortcuts"), scObj}});
            applyDynamicShortcuts();
        });
        shortcutEdits_.insert(k, ed);
        scGrid->addWidget(ed, r, c + 1);
        ++sIdx;
    }
    scLay->addLayout(scGrid);
    bodyLay->addWidget(scCard);

    scroll->setWidget(body);
    advRoot->addWidget(scroll, 1);
}

void StudioController::applyThemeAndFontAll(bool saveSettings) {
    auto* app = qobject_cast<QApplication*>(QApplication::instance());
    if (app) {
        applyGlobalFontScale(app, fontPct_);
        app->setPalette(buildSalsaPalette(currentTheme_));
        app->setStyleSheet(buildSalsaStylesheet(currentTheme_, false, fontPct_));
    }

    if (lblVerPill_) {
        lblVerPill_->setStyleSheet(
            QStringLiteral("background-color: %1; color: %2; border: 1px solid %2; border-radius: 5px; padding: 2px 8px; font-weight: 800; font-size: %3px;")
                .arg(currentTheme_.color("bg_deep", "#0c0e11"),
                     currentTheme_.color("accent_warn", "#f59e0b"),
                     QString::number(scaledPx(11, fontPct_))));
    }

    if (spectrum_) spectrum_->setTheme(currentTheme_, fontPct_);
    if (graphCanvas_) graphCanvas_->setTheme(currentTheme_, fontPct_);
    for (QSlider* bs : bandSliders_) {
        if (bs) bs->setMinimumHeight(scaledPx(112, fontPct_));
    }
    auto measureLabelWidth = [&](QLabel* lbl, const QString& sample, int floorPx, int pad) {
        if (!lbl) return floorPx;
        lbl->ensurePolished();
        const QFontMetrics appFm(app ? app->font() : lbl->font());
        const int textPx = std::max(lbl->fontMetrics().horizontalAdvance(sample),
                                    appFm.horizontalAdvance(sample));
        return std::max(floorPx, textPx + pad);
    };
    const int masterLblW = std::max(
        measureLabelWidth(lblSpk_, QStringLiteral("Speaker Vol (150%):"), scaledPx(140, fontPct_), 12),
        measureLabelWidth(lblMic_, QStringLiteral("Mic Capture (150%):"), scaledPx(140, fontPct_), 12));
    if (lblSpk_) lblSpk_->setFixedWidth(masterLblW);
    if (lblMic_) lblMic_->setFixedWidth(masterLblW);
    if (lblCrossfeed_) {
        lblCrossfeed_->setFixedWidth(
            measureLabelWidth(lblCrossfeed_, QStringLiteral("100%"), scaledPx(44, fontPct_), 10));
    }
    if (lblHwBoost_) {
        lblHwBoost_->setFixedWidth(
            measureLabelWidth(lblHwBoost_, QStringLiteral("+24.0 dB"), scaledPx(76, fontPct_), 14));
    }
    if (lblHwMicBoost_) {
        lblHwMicBoost_->setFixedWidth(
            measureLabelWidth(lblHwMicBoost_, QStringLiteral("+24.0 dB"), scaledPx(76, fontPct_), 14));
    }
    if (lblPreamp_ && lblThresh_ && lblRatio_) {
        const QFontMetrics dfm(app ? app->font() : lblPreamp_->font());
        lblPreamp_->setFixedWidth(dfm.horizontalAdvance(QStringLiteral("Preamp: -18.0 dB")) + 10);
        lblThresh_->setFixedWidth(dfm.horizontalAdvance(QStringLiteral("Dyn Threshold: -48.0 dB")) + 10);
        lblRatio_->setFixedWidth(dfm.horizontalAdvance(QStringLiteral("Dyn Ratio: 8.0:1")) + 10);
    }
    if (lblPeaks_) {
        QFont pfont(QStringLiteral("Monospace"));
        pfont.setBold(true);
        pfont.setPixelSize(scaledPx(12, fontPct_));
        const QFontMetrics pfm(pfont);
        lblPeaks_->setFixedWidth(
            pfm.horizontalAdvance(QStringLiteral("Output Peak L/R: -100.0 / -100.0 dBFS")) + 10);
    }
    if (tblParamBands_) {
        tblParamBands_->verticalHeader()->setDefaultSectionSize(scaledPx(26, fontPct_));
    }
    if (tblSessions_) {
        tblSessions_->setColumnWidth(5, scaledPx(96, fontPct_));
        tblSessions_->setColumnWidth(7, scaledPx(126, fontPct_));
        tblSessions_->verticalHeader()->setDefaultSectionSize(scaledPx(32, fontPct_));
    }
    selectManualChapter(currentManualKey_);
    if (advWin_ && advWin_->isVisible()) refreshAdvIntegration();

    if (saveSettings) {
        salsa_save_font_scale(fontPct_);
        updateGuiSettingsPatch(QJsonObject{
            {QStringLiteral("theme"), currentThemeName_},
        });
        lastSettingsMtimeNs_ = salsa_gui_settings_mtime_ns();
    }
}

void StudioController::setFontScalePctAndSync(int pct) {
    fontPct_ = normalizeFontScalePct(pct);
    salsa_save_font_scale(fontPct_);
    lastSettingsMtimeNs_ = salsa_gui_settings_mtime_ns();
    syncFontScaleBars();
    applyThemeAndFontAll(false);
}

void StudioController::syncFontScaleBars() {
    if (fontBarMain_) fontBarMain_->setFontScalePct(fontPct_);
    if (fontBarIntro_) fontBarIntro_->setFontScalePct(fontPct_);
    if (fontBarAdv_) fontBarAdv_->setFontScalePct(fontPct_);
}

void StudioController::applyDynamicShortcuts() {
    for (QShortcut* sc : activeShortcuts_) sc->deleteLater();
    activeShortcuts_.clear();

    auto addSc = [this](const QString& key, std::function<void()> fn) {
        const QString seq = shortcutsMap_.value(key);
        if (seq.isEmpty()) return;
        for (QWidget* host : std::array<QWidget*, 3>{mainWin_, introWin_, advWin_}) {
            if (!host) continue;
            auto* sc = new QShortcut(QKeySequence(seq), host);
            sc->setContext(Qt::WindowShortcut);
            connect(sc, &QShortcut::activated, this, fn);
            activeShortcuts_.push_back(sc);
        }
    };

    addSc(QStringLiteral("topology"), [this]() { openTopologyWindow(); });
    addSc(QStringLiteral("advanced"), [this]() { openAdvancedWindow(); });
    addSc(QStringLiteral("debugger"), [this]() { openLiveDebuggerWindow(); });
    addSc(QStringLiteral("vol_up"), [this]() {
        if (!sldSpk_) return;
        sldSpk_->setValue(std::clamp(sldSpk_->value() + volStepPct_, 0, 150));
        spkSendTimer_->start(0);
    });
    addSc(QStringLiteral("vol_down"), [this]() {
        if (!sldSpk_) return;
        sldSpk_->setValue(std::clamp(sldSpk_->value() - volStepPct_, 0, 150));
        spkSendTimer_->start(0);
    });
    addSc(QStringLiteral("mic_up"), [this]() {
        if (!sldMic_) return;
        sldMic_->setValue(std::clamp(sldMic_->value() + volStepPct_, 0, 150));
        micSendTimer_->start(0);
    });
    addSc(QStringLiteral("mic_down"), [this]() {
        if (!sldMic_) return;
        sldMic_->setValue(std::clamp(sldMic_->value() - volStepPct_, 0, 150));
        micSendTimer_->start(0);
    });
    addSc(QStringLiteral("mute_spk"), [this]() { if (btnSpkMute_) btnSpkMute_->click(); });
    addSc(QStringLiteral("mute_mic"), [this]() { if (btnMicMute_) btnMicMute_->click(); });
    addSc(QStringLiteral("toggle_eq"), [this]() { if (chkEqEnable_) chkEqEnable_->toggle(); });
    addSc(QStringLiteral("bt_reset"), [this]() { onBtResetClicked(); });
    addSc(QStringLiteral("save_config"), [this]() { onSaveOverwriteConfigClicked(); });
    addSc(QStringLiteral("refresh"), [this]() { pollEngine(false); });
    addSc(QStringLiteral("quit"), [this]() { shutdownAll(); });
    addSc(QStringLiteral("reset_flat"), [this]() { onResetFlatClicked(); });

    // Universal Zoom shortcuts across Studio windows
    for (QWidget* host : std::array<QWidget*, 3>{mainWin_, introWin_, advWin_}) {
        if (!host) continue;
        for (const QKeySequence& seq : {QKeySequence(QKeySequence::ZoomIn), QKeySequence(Qt::CTRL | Qt::Key_Equal)}) {
            auto* scPlus = new QShortcut(seq, host);
            connect(scPlus, &QShortcut::activated, this, [this]() { setFontScalePctAndSync(fontPct_ + 10); });
            activeShortcuts_.push_back(scPlus);
        }
        for (const QKeySequence& seq : {QKeySequence(QKeySequence::ZoomOut), QKeySequence(Qt::CTRL | Qt::Key_Minus)}) {
            auto* scMinus = new QShortcut(seq, host);
            connect(scMinus, &QShortcut::activated, this, [this]() { setFontScalePctAndSync(fontPct_ - 10); });
            activeShortcuts_.push_back(scMinus);
        }
        auto* scReset = new QShortcut(QKeySequence(QStringLiteral("Ctrl+0")), host);
        connect(scReset, &QShortcut::activated, this, [this]() { setFontScalePctAndSync(100); });
        activeShortcuts_.push_back(scReset);

        if (host == introWin_) {
            auto* scRet = new QShortcut(QKeySequence(Qt::Key_Return), introWin_);
            connect(scRet, &QShortcut::activated, this, [this]() { enterStudioFromTopology(); });
            activeShortcuts_.push_back(scRet);
            auto* scEsc = new QShortcut(QKeySequence(Qt::Key_Escape), introWin_);
            connect(scEsc, &QShortcut::activated, this, [this]() { enterStudioFromTopology(); });
            activeShortcuts_.push_back(scEsc);
        }
    }
}

void StudioController::populatePresetDropdown(const QString& selectTarget) {
    if (!cmbPreset_) return;
    cmbPreset_->blockSignals(true);
    cmbPreset_->clear();
    for (const QJsonValue& pv : builtinEqPresets_) {
        const QString name = pv.toObject().value(QStringLiteral("name")).toString();
        if (!name.isEmpty()) cmbPreset_->addItem(name, QStringLiteral("builtin:") + name);
    }
    for (const QJsonValue& av : savedAutoEq_) {
        const QJsonObject ao = av.toObject();
        const QString name = ao.value(QStringLiteral("name")).toString();
        const QString src = ao.value(QStringLiteral("source_path")).toString();
        if (!name.isEmpty()) {
            cmbPreset_->addItem(QStringLiteral("[AutoEQ] %1").arg(name), QStringLiteral("autoeq:") + name);
            if (!src.isEmpty()) {
                cmbPreset_->setItemData(cmbPreset_->count() - 1, QStringLiteral("AutoEQ file: %1").arg(src), Qt::ToolTipRole);
            }
        }
    }
    if (!selectTarget.isEmpty()) {
        int idx = cmbPreset_->findData(QStringLiteral("autoeq:") + selectTarget);
        if (idx < 0) idx = cmbPreset_->findData(QStringLiteral("builtin:") + selectTarget);
        if (idx >= 0) cmbPreset_->setCurrentIndex(idx);
    }
    cmbPreset_->blockSignals(false);
}

void StudioController::onThemeActivated(int index) {
    if (index < 0 || !cmbTheme_) return;
    const QString name = cmbTheme_->itemText(index);
    if (name == QLatin1String(CUSTOM_THEME_NAME)) {
        openCustomThemeDialog();
        return;
    }
    currentThemeName_ = name;
    currentTheme_ = themeByName(name);
    applyThemeAndFontAll(true);
}

void StudioController::openCustomThemeDialog() {
    if (!themeDlg_) {
        QWidget* dlgParent = (mainWin_ && mainWin_->isVisible()) ? static_cast<QWidget*>(mainWin_) : nullptr;
        themeDlg_ = new SalsaCustomThemeDialog(currentTheme_, fontPct_, dlgParent);
        connect(themeDlg_, &SalsaCustomThemeDialog::liveThemeChanged, this, [this](const SalsaThemeData& t) {
            currentTheme_ = t;
            applyThemeAndFontAll(false);
        });
        connect(themeDlg_, &SalsaCustomThemeDialog::mirrorStopped, this, [this]() {
            currentTheme_ = themeByName(currentThemeName_);
            applyThemeAndFontAll(false);
        });
        connect(themeDlg_, &SalsaCustomThemeDialog::themeApplied, this, [this](const SalsaThemeData& t) {
            currentTheme_ = t;
            currentTheme_.name = QString::fromLatin1(CUSTOM_THEME_NAME);
            currentThemeName_ = QString::fromLatin1(CUSTOM_THEME_NAME);
            if (cmbTheme_) {
                cmbTheme_->blockSignals(true);
                cmbTheme_->setCurrentText(currentThemeName_);
                cmbTheme_->blockSignals(false);
            }
            updateGuiSettingsPatch(QJsonObject{
                {QStringLiteral("theme"), currentThemeName_},
                {QStringLiteral("custom_theme"), themeToJsonObj(currentTheme_)},
            });
            applyThemeAndFontAll(true);
        });
        connect(themeDlg_, &QDialog::rejected, this, [this]() {
            if (cmbTheme_) {
                cmbTheme_->blockSignals(true);
                cmbTheme_->setCurrentText(currentThemeName_);
                cmbTheme_->blockSignals(false);
            }
        });
        connect(themeDlg_, &QDialog::finished, this, [this](int) {
            if (closingAll_) return;
            if ((!mainWin_ || !mainWin_->isVisible())
                && (!introWin_ || !introWin_->isVisible())
                && (!advWin_ || !advWin_->isVisible())) {
                closingAll_ = true;
                QTimer::singleShot(0, qApp, &QCoreApplication::quit);
            }
        });
    } else {
        themeDlg_->resetForOpen(currentTheme_, fontPct_);
    }
    themeDlg_->show();
    themeDlg_->raise();
    themeDlg_->activateWindow();
}

void StudioController::pushEqToEngine() {
    eqHoldUntilMs_ = nowMs() + 650;
    eqCfg_.enabled = chkEqEnable_->isChecked() ? 1 : 0;
    eqCfg_.dynamic_mode = static_cast<uint32_t>(cmbDynMode_->currentData().toInt());
    eqCfg_.limiter_enabled = chkLimiter_->isChecked() ? 1 : 0;
    eqCfg_.preamp_db = sldPreamp_->value() / 10.0f;
    eqCfg_.dyn_threshold_db = sldThresh_->value() / 10.0f;
    eqCfg_.dyn_ratio = sldRatio_->value() / 10.0f;
    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
        eqCfg_.band_gains_db[i] = bandSliders_[i]->value() / 10.0f;
    }
    paramExt_.crossfeed_enabled = chkCrossfeed_->isChecked() ? 1 : 0;
    paramExt_.crossfeed_level = sldCrossfeed_->value() / 100.0f;
    paramExt_.crossfeed_cutoff_hz = static_cast<uint32_t>(qvarInt(cmbCrossfeedCut_->currentData(), 700));

    salsa_set_parametric_eq(&eqCfg_, &paramExt_);
    updateSpectrumFromState();
    if (settingsSaveTimer_) {
        settingsSaveTimer_->start();
    } else {
        saveStudioSettings();
    }
}

void StudioController::saveStudioSettings() {
    const QJsonObject eqObj = parseJsonObj(SalsaStr(salsa_eq_cfg_to_json(&eqCfg_, &paramExt_)).toQString());
    QJsonObject payload;
    payload.insert(QStringLiteral("eq"), eqObj);
    payload.insert(QStringLiteral("imported_eq_presets"), savedAutoEq_);
    payload.insert(QStringLiteral("eq_per_device"), chkEqPerDev_->isChecked());
    payload.insert(QStringLiteral("theme"), currentThemeName_);
    payload.insert(QStringLiteral("font_scale_pct"), fontPct_);
    payload.insert(QStringLiteral("volume_step_pct"), volStepPct_);
    for (auto it = payload.constBegin(); it != payload.constEnd(); ++it) {
        guiSettings_.insert(it.key(), it.value());
    }
    if (chkEqPerDev_->isChecked() && !lastSinkId_.isEmpty()) {
        savePerDeviceEqForSink(lastSinkId_);
    }
    salsa_save_studio_settings_json(jsonCompact(payload).toUtf8().constData());
    lastSettingsMtimeNs_ = salsa_gui_settings_mtime_ns();
}

void StudioController::applyGuiSettingsJsonToUi(bool isReload) {
    updatingUi_ = true;

    if (isReload) {
        fontPct_ = normalizeFontScalePct(salsa_load_font_scale());
        volStepPct_ = normalizeVolStepPct(guiSettings_.value(QStringLiteral("volume_step_pct")).toInt(5));
        if (cmbVolStep_) {
            if (int si = cmbVolStep_->findData(volStepPct_); si >= 0) cmbVolStep_->setCurrentIndex(si);
        }
        syncFontScaleBars();
        savedAutoEq_ = parseJsonArr(SalsaStr(salsa_saved_autoeq_list_json()).toQString());
        if (chkEqPerDev_) {
            chkEqPerDev_->setChecked(guiSettings_.value(QStringLiteral("eq_per_device")).toBool(false));
        }
        const bool showIntro = guiSettings_.value(QStringLiteral("show_topology_intro")).toBool(true);
        if (chkShowIntroAdv_) chkShowIntroAdv_->setChecked(showIntro);
        if (chkHideIntroStartup_) chkHideIntroStartup_->setChecked(!showIntro);

        const SalsaThemeData diskTheme = loadThemeForApp(false);
        const QString diskThemeName = !diskTheme.name.isEmpty()
            ? diskTheme.name
            : guiSettings_.value(QStringLiteral("theme")).toString(currentThemeName_);
        currentTheme_ = diskTheme;
        currentThemeName_ = diskThemeName;
        if (cmbTheme_) cmbTheme_->setCurrentText(currentThemeName_);

        const QJsonArray defSc = parseJsonArr(SalsaStr(salsa_default_shortcuts_json()).toQString());
        const QJsonObject savedSc = guiSettings_.value(QStringLiteral("shortcuts")).toObject();
        shortcutsMap_.clear();
        for (const QJsonValue& v : defSc) {
            const QJsonObject o = v.toObject();
            const QString k = o.value(QStringLiteral("key")).toString();
            const QString defSeq = o.value(QStringLiteral("default_seq")).toString();
            if (!k.isEmpty()) shortcutsMap_.insert(k, savedSc.value(k).toString(defSeq));
        }
    }

    // Restore Engine clock/capture/targets from unified "engine" object (with legacy fallback)
    const QJsonObject eng = guiSettings_.value(QStringLiteral("engine")).toObject();
    const int sr = eng.contains(QStringLiteral("sample_rate"))
        ? eng.value(QStringLiteral("sample_rate")).toInt(48000)
        : guiSettings_.value(QStringLiteral("rate")).toInt(48000);
    const int pf = eng.contains(QStringLiteral("period_frames"))
        ? eng.value(QStringLiteral("period_frames")).toInt(256)
        : guiSettings_.value(QStringLiteral("period")).toInt(256);
    const int fb = eng.contains(QStringLiteral("format_bits"))
        ? eng.value(QStringLiteral("format_bits")).toInt(32)
        : guiSettings_.value(QStringLiteral("bits")).toInt(32);
    const int csr = eng.contains(QStringLiteral("capture_sample_rate"))
        ? eng.value(QStringLiteral("capture_sample_rate")).toInt(0)
        : guiSettings_.value(QStringLiteral("capture_sample_rate")).toInt(0);
    const QString savedPb = eng.value(QStringLiteral("target_playback_id")).toString(QStringLiteral("auto")).trimmed();
    const QString savedCap = eng.value(QStringLiteral("target_capture_id")).toString(QStringLiteral("auto")).trimmed();
    targetPbId_ = savedPb.isEmpty() ? QStringLiteral("auto") : savedPb;
    targetCapId_ = savedCap.isEmpty() ? QStringLiteral("auto") : savedCap;
    if (cmbRate_) {
        if (int ri = cmbRate_->findData(sr); ri >= 0) cmbRate_->setCurrentIndex(ri);
    }
    if (cmbPeriod_) {
        if (int pi = cmbPeriod_->findData(pf); pi >= 0) cmbPeriod_->setCurrentIndex(pi);
    }
    if (cmbBits_) {
        if (int bi = cmbBits_->findData(fb); bi >= 0) cmbBits_->setCurrentIndex(bi);
    }
    if (cmbCapRate_) {
        if (int ci = cmbCapRate_->findData(csr); ci >= 0) cmbCapRate_->setCurrentIndex(ci);
    }
    if (cmbSink_) {
        if (int si = cmbSink_->findData(targetPbId_); si >= 0) cmbSink_->setCurrentIndex(si);
    }
    if (cmbSource_) {
        if (int si = cmbSource_->findData(targetCapId_); si >= 0) cmbSource_->setCurrentIndex(si);
    }

    // Restore Volumes
    if (guiSettings_.contains(QStringLiteral("volumes"))) {
        const QJsonObject vol = guiSettings_.value(QStringLiteral("volumes")).toObject();
        const int mp = std::clamp(vol.value(QStringLiteral("master_pct")).toInt(100), 0, 150);
        const bool mm = vol.value(QStringLiteral("master_muted")).toBool(false);
        const int micp = std::clamp(vol.value(QStringLiteral("mic_pct")).toInt(100), 0, 150);
        const bool micm = vol.value(QStringLiteral("mic_muted")).toBool(false);
        if (sldSpk_) sldSpk_->setValue(mp);
        if (lblSpk_) lblSpk_->setText(QStringLiteral("Speaker Vol (%1%):").arg(mp));
        if (btnSpkMute_) btnSpkMute_->setText(mm ? QStringLiteral("Unmute Spk") : QStringLiteral("Mute Spk"));
        if (sldMic_) sldMic_->setValue(micp);
        if (lblMic_) lblMic_->setText(QStringLiteral("Mic Capture (%1%):").arg(micp));
        if (btnMicMute_) btnMicMute_->setText(micm ? QStringLiteral("Unmute Mic") : QStringLiteral("Mute Mic"));
    }

    // Restore Bluetooth preferences
    if (guiSettings_.contains(QStringLiteral("bluetooth"))) {
        const QJsonObject bt = guiSettings_.value(QStringLiteral("bluetooth")).toObject();
        btCfg_.preferred_codec = static_cast<uint32_t>(std::clamp(bt.value(QStringLiteral("preferred_codec")).toInt(0), 0, 5));
        btCfg_.ldac_mode = static_cast<uint32_t>(std::clamp(bt.value(QStringLiteral("ldac_mode")).toInt(0), 0, 3));
        btCfg_.wireless_abr_enabled = bt.value(QStringLiteral("wireless_abr_enabled")).toBool(true) ? 1 : 0;
        btCfg_.hfp_sco_enabled = bt.value(QStringLiteral("hfp_sco_enabled")).toBool(false) ? 1 : 0;
        if (cmbBtCodec_) {
            if (int ci = cmbBtCodec_->findData(static_cast<int>(btCfg_.preferred_codec)); ci >= 0) cmbBtCodec_->setCurrentIndex(ci);
        }
        if (cmbBtLdac_) {
            if (int li = cmbBtLdac_->findData(static_cast<int>(btCfg_.ldac_mode)); li >= 0) cmbBtLdac_->setCurrentIndex(li);
        }
        if (chkBtAbr_) chkBtAbr_->setChecked(btCfg_.wireless_abr_enabled != 0);
        if (chkBtSco_) chkBtSco_->setChecked(btCfg_.hfp_sco_enabled != 0);
        if (chkBtOptional_) chkBtOptional_->setChecked(bt.value(QStringLiteral("optional_mode")).toBool(true));
    }

    // Restore Hardware Analog Stage preferences
    if (guiSettings_.contains(QStringLiteral("hw_analog"))) {
        const QJsonObject hwa = guiSettings_.value(QStringLiteral("hw_analog")).toObject();
        hwAnalog_.hw_dac_unity_vol = hwa.value(QStringLiteral("hw_dac_unity_vol")).toBool(true) ? 1 : 0;
        hwAnalog_.hp_amp_tier = static_cast<uint32_t>(std::clamp(hwa.value(QStringLiteral("hp_amp_tier")).toInt(0), 0, 3));
        hwAnalog_.output_route = static_cast<uint32_t>(std::clamp(hwa.value(QStringLiteral("output_route")).toInt(0), 0, 2));
        hwAnalog_.unlock_0db_rails = hwa.value(QStringLiteral("unlock_0db_rails")).toBool(false) ? 1 : 0;
        hwAnalog_.analog_boost_db = static_cast<float>(std::clamp(hwa.value(QStringLiteral("analog_boost_db")).toDouble(0.0), 0.0, 24.0));
        hwAnalog_.mic_hw_boost_db = static_cast<float>(std::clamp(hwa.value(QStringLiteral("mic_hw_boost_db")).toDouble(0.0), 0.0, 24.0));
        if (chkHwDacUnity_) chkHwDacUnity_->setChecked(hwAnalog_.hw_dac_unity_vol != 0);
        if (cmbHwAmp_) {
            if (int ai = cmbHwAmp_->findData(static_cast<int>(hwAnalog_.hp_amp_tier)); ai >= 0) cmbHwAmp_->setCurrentIndex(ai);
        }
        if (cmbHwRoute_) {
            if (int ri = cmbHwRoute_->findData(static_cast<int>(hwAnalog_.output_route)); ri >= 0) cmbHwRoute_->setCurrentIndex(ri);
        }
        if (chkHwRails_) chkHwRails_->setChecked(hwAnalog_.unlock_0db_rails != 0);
        if (sldHwBoost_) sldHwBoost_->setValue(static_cast<int>(std::lround(hwAnalog_.analog_boost_db * 2.0f)));
        if (lblHwBoost_) lblHwBoost_->setText(formatBoostDbText(hwAnalog_.analog_boost_db));
        if (sldHwMicBoost_) sldHwMicBoost_->setValue(static_cast<int>(std::lround(hwAnalog_.mic_hw_boost_db)));
        if (lblHwMicBoost_) lblHwMicBoost_->setText(formatBoostDbText(hwAnalog_.mic_hw_boost_db));
    }

    // Restore EQ & parametric bands
    if (guiSettings_.contains(QStringLiteral("eq")) || guiSettings_.contains(QStringLiteral("eq_config"))) {
        const QString key = guiSettings_.contains(QStringLiteral("eq"))
            ? QStringLiteral("eq")
            : QStringLiteral("eq_config");
        const QByteArray eqJs = jsonCompact(guiSettings_.value(key).toObject()).toUtf8();
        salsa_json_to_eq_cfg(eqJs.constData(), &eqCfg_, &paramExt_);
    }

    chkEqEnable_->setChecked(eqCfg_.enabled != 0);
    if (int di = cmbDynMode_->findData(static_cast<int>(eqCfg_.dynamic_mode)); di >= 0) {
        cmbDynMode_->setCurrentIndex(di);
    }
    lblSavedDynMode_->setText(dynModeSavedLabel(eqCfg_.dynamic_mode));
    sldPreamp_->setValue(static_cast<int>(std::lround(eqCfg_.preamp_db * 10.0)));
    sldThresh_->setValue(static_cast<int>(std::lround(eqCfg_.dyn_threshold_db * 10.0)));
    sldRatio_->setValue(static_cast<int>(std::lround(eqCfg_.dyn_ratio * 10.0)));
    chkLimiter_->setChecked(eqCfg_.limiter_enabled != 0);
    chkCrossfeed_->setChecked(paramExt_.crossfeed_enabled != 0);
    sldCrossfeed_->setValue(static_cast<int>(std::lround(paramExt_.crossfeed_level * 100.0)));
    if (lblCrossfeed_) lblCrossfeed_->setText(formatPctText(sldCrossfeed_->value()));
    if (int ci = cmbCrossfeedCut_->findData(static_cast<int>(paramExt_.crossfeed_cutoff_hz)); ci >= 0) {
        cmbCrossfeedCut_->setCurrentIndex(ci);
    }
    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
        bandSliders_[i]->setValue(static_cast<int>(std::lround(eqCfg_.band_gains_db[i] * 10.0)));
        bandValLbls_[i]->setText(QStringLiteral("%1%2 dB").arg(eqCfg_.band_gains_db[i] >= 0 ? QStringLiteral("+") : QString()).arg(eqCfg_.band_gains_db[i], 0, 'f', 1));
        bandFreqLbls_[i]->setText(fmtFreqHz(paramExt_.band_freq_hz[i]));
    }
    populatePresetDropdown(cFixedStr(eqCfg_.preset_name, sizeof(eqCfg_.preset_name)));
    lblSavedPreset_->setText(QStringLiteral("Saved: %1").arg(cFixedStr(eqCfg_.preset_name, sizeof(eqCfg_.preset_name))));
    lblPreamp_->setText(QStringLiteral("Preamp: %1%2 dB").arg(eqCfg_.preamp_db >= 0 ? QStringLiteral("+") : QString()).arg(eqCfg_.preamp_db, 0, 'f', 1));
    lblThresh_->setText(QStringLiteral("Dyn Threshold: %1 dB").arg(eqCfg_.dyn_threshold_db, 0, 'f', 1));
    lblRatio_->setText(QStringLiteral("Dyn Ratio: %1:1").arg(eqCfg_.dyn_ratio, 0, 'f', 1));
    updateBandParamTable();
    updatingUi_ = false;
    updateSpectrumFromState();

    if (isReload) {
        applyThemeAndFontAll(false);
        applyDynamicShortcuts();
    }
}

void StudioController::flashSaveFeedback(const QString& statusMsg, bool ok) {
    if (lblCfgSaveStatus_) {
        lblCfgSaveStatus_->setText(statusMsg);
        lblCfgSaveStatus_->setToolTip(statusMsg);
    }
    const QString flashLabel = ok ? QStringLiteral("Saved \u2713") : QStringLiteral("Save Failed");
    auto flashBtn = [this, &flashLabel](QPushButton* btn, const QString& restoreText) {
        if (!btn) return;
        const int prevMinW = btn->minimumWidth();
        const int holdW = std::max(btn->width(), btn->sizeHint().width());
        if (holdW > 0) btn->setMinimumWidth(holdW);
        btn->setText(flashLabel);
        QPointer<QPushButton> pBtn(btn);
        QTimer::singleShot(1800, this, [pBtn, restoreText, prevMinW]() {
            if (!pBtn) return;
            pBtn->setText(restoreText);
            pBtn->setMinimumWidth(prevMinW);
        });
    };
    flashBtn(btnSaveCfgMain_, QStringLiteral("Save Settings (Ctrl+S)"));
    flashBtn(btnSaveCfgAdv_, QStringLiteral("Save Settings"));
    flashBtn(btnSaveHw_, QStringLiteral("Save Settings"));
}

void StudioController::onSaveOverwriteConfigClicked() {
    if (settingsSaveTimer_ && settingsSaveTimer_->isActive()) settingsSaveTimer_->stop();
    if (hwSendTimer_ && hwSendTimer_->isActive()) hwSendTimer_->stop();
    if (spkSendTimer_ && spkSendTimer_->isActive()) spkSendTimer_->stop();
    if (micSendTimer_ && micSendTimer_->isActive()) micSendTimer_->stop();

    const qint64 holdUntil = nowMs() + 1400;
    eqHoldUntilMs_ = holdUntil;
    btHoldUntilMs_ = holdUntil;
    hwHoldUntilMs_ = holdUntil;
    spkHoldUntilMs_ = holdUntil;
    micHoldUntilMs_ = holdUntil;

    const uint32_t rate = static_cast<uint32_t>(qvarInt(cmbRate_->currentData(), 48000));
    const uint32_t period = static_cast<uint32_t>(qvarInt(cmbPeriod_->currentData(), 256));
    const uint32_t bits = static_cast<uint32_t>(qvarInt(cmbBits_->currentData(), 32));
    const uint32_t capRate = static_cast<uint32_t>(qvarInt(cmbCapRate_->currentData(), 0));
    const uint32_t masterPct = static_cast<uint32_t>(std::clamp(sldSpk_->value(), 0, 150));
    const uint32_t masterMuted = btnSpkMute_->text().startsWith(QStringLiteral("Unmute")) ? 1U : 0U;
    const uint32_t micPct = static_cast<uint32_t>(std::clamp(sldMic_->value(), 0, 150));
    const uint32_t micMuted = btnMicMute_->text().startsWith(QStringLiteral("Unmute")) ? 1U : 0U;
    const uint32_t btOpt = (chkBtOptional_ && chkBtOptional_->isChecked()) ? 1U : 0U;

    btCfg_.preferred_codec = static_cast<uint32_t>(cmbBtCodec_->currentData().toInt());
    btCfg_.ldac_mode = static_cast<uint32_t>(cmbBtLdac_->currentData().toInt());
    btCfg_.wireless_abr_enabled = chkBtAbr_->isChecked() ? 1 : 0;
    btCfg_.hfp_sco_enabled = chkBtSco_->isChecked() ? 1 : 0;

    hwAnalog_.hw_dac_unity_vol = chkHwDacUnity_->isChecked() ? 1 : 0;
    hwAnalog_.hp_amp_tier = static_cast<uint32_t>(cmbHwAmp_->currentData().toInt());
    hwAnalog_.output_route = static_cast<uint32_t>(cmbHwRoute_->currentData().toInt());
    hwAnalog_.unlock_0db_rails = chkHwRails_->isChecked() ? 1 : 0;
    hwAnalog_.analog_boost_db = sldHwBoost_->value() * 0.5f;
    hwAnalog_.mic_hw_boost_db = static_cast<float>(sldHwMicBoost_->value());

    eqCfg_.enabled = chkEqEnable_->isChecked() ? 1 : 0;
    eqCfg_.limiter_enabled = chkLimiter_->isChecked() ? 1 : 0;
    paramExt_.crossfeed_enabled = chkCrossfeed_->isChecked() ? 1 : 0;

    if (chkEqPerDev_->isChecked() && !lastSinkId_.isEmpty()) {
        savePerDeviceEqForSink(lastSinkId_);
    }

    // Push live changes to daemon if online so both in-memory engine and disk config match
    SalsaEngineStatus curSt{};
    const bool daemonOnline = (salsa_get_engine_status(&curSt) == 0);
    if (daemonOnline) {
        if (rate != curSt.hw_sample_rate || period != curSt.hw_period_frames || bits != curSt.hw_format_bits) {
            salsa_set_engine_config(rate, period, bits);
        }
        salsa_set_capture_sample_rate(capRate);
        salsa_set_master_volume(masterPct, masterMuted);
        salsa_set_mic_volume(micPct, micMuted);
        salsa_set_bt_codec_config(&btCfg_);
        uint32_t flagsOut = 0;
        salsa_bt_connection_ctl(btOpt ? SALSA_BT_CONN_SET_OPTIONAL : SALSA_BT_CONN_SET_AUTOCONN, "", &flagsOut);
        salsa_set_hw_analog(&hwAnalog_);
        salsa_set_parametric_eq(&eqCfg_, &paramExt_);
        salsa_save_config_now();
    }

    QJsonObject guiPayload;
    guiPayload.insert(QStringLiteral("imported_eq_presets"), savedAutoEq_);
    guiPayload.insert(QStringLiteral("eq_per_device"), chkEqPerDev_->isChecked());
    if (chkShowIntroAdv_) {
        guiPayload.insert(QStringLiteral("show_topology_intro"), chkShowIntroAdv_->isChecked());
    }
    guiPayload.insert(QStringLiteral("theme"), currentThemeName_);
    guiPayload.insert(QStringLiteral("font_scale_pct"), fontPct_);
    guiPayload.insert(QStringLiteral("volume_step_pct"), volStepPct_);
    if (guiSettings_.contains(QStringLiteral("per_device_eq"))) {
        guiPayload.insert(QStringLiteral("per_device_eq"), guiSettings_.value(QStringLiteral("per_device_eq")));
    }

    const QByteArray pbBytes = targetPbId_.toUtf8();
    const QByteArray capBytes = targetCapId_.toUtf8();
    const QByteArray guiBytes = jsonCompact(guiPayload).toUtf8();
    const int rc = salsa_overwrite_settings_from_ui(
        rate,
        period,
        bits,
        capRate,
        pbBytes.constData(),
        capBytes.constData(),
        masterPct,
        masterMuted,
        micPct,
        micMuted,
        btOpt,
        &btCfg_,
        &hwAnalog_,
        &eqCfg_,
        &paramExt_,
        guiBytes.constData());

    if (rc == 0) {
        guiSettings_ = loadGuiSettings();
        lastSettingsMtimeNs_ = salsa_gui_settings_mtime_ns();
        const QString path = SalsaStr(salsa_settings_path_str()).toQString();
        const QString msg = daemonOnline
            ? QStringLiteral("Saved (%2): %1")
                  .arg(path, QDateTime::currentDateTime().toString(QStringLiteral("HH:mm:ss")))
            : QStringLiteral("Saved offline (%2): %1")
                  .arg(path, QDateTime::currentDateTime().toString(QStringLiteral("HH:mm:ss")));
        flashSaveFeedback(msg, true);
    } else {
        flashSaveFeedback(QStringLiteral("Error: could not write settings (check 0700/0600 ownership & permissions)"), false);
    }
}

void StudioController::onReloadConfigClicked() {
    if (settingsSaveTimer_ && settingsSaveTimer_->isActive()) settingsSaveTimer_->stop();
    if (hwSendTimer_ && hwSendTimer_->isActive()) hwSendTimer_->stop();
    if (spkSendTimer_ && spkSendTimer_->isActive()) spkSendTimer_->stop();
    if (micSendTimer_ && micSendTimer_->isActive()) micSendTimer_->stop();

    eqHoldUntilMs_ = 0;
    btHoldUntilMs_ = 0;
    hwHoldUntilMs_ = 0;
    spkHoldUntilMs_ = 0;
    micHoldUntilMs_ = 0;

    const int engineRc = salsa_reload_config_now();
    guiSettings_ = loadGuiSettings();
    lastSettingsMtimeNs_ = salsa_gui_settings_mtime_ns();
    applyGuiSettingsJsonToUi(true);
    pollEngine(true);

    const QString path = SalsaStr(salsa_settings_path_str()).toQString();
    if (lblCfgSaveStatus_) {
        const QString msg = (engineRc == 0)
            ? QStringLiteral("Reloaded daemon + UI from %1 (%2)")
                  .arg(path, QDateTime::currentDateTime().toString(QStringLiteral("HH:mm:ss")))
            : QStringLiteral("Reloaded UI from %1 (engine offline)")
                  .arg(path);
        lblCfgSaveStatus_->setText(msg);
        lblCfgSaveStatus_->setToolTip(msg);
    }
}

void StudioController::onOpenConfigFileClicked() {
    const QString cfgPath = SalsaStr(salsa_settings_path_str()).toQString();
    QFileInfo fi(cfgPath);
    if (!fi.exists()) {
        saveStudioSettings();
        fi.refresh();
    }
    if (fi.isSymLink()) {
        if (lblCfgSaveStatus_) {
            lblCfgSaveStatus_->setText(QStringLiteral("Blocked opening symlinked config path: %1").arg(cfgPath));
        }
        return;
    }

    if (fi.exists() && QDesktopServices::openUrl(QUrl::fromLocalFile(cfgPath))) {
        if (lblCfgSaveStatus_) {
            const QString msg = QStringLiteral("Opened %1 (click Reload Saved Config after external edits)").arg(cfgPath);
            lblCfgSaveStatus_->setText(msg);
            lblCfgSaveStatus_->setToolTip(msg);
        }
        return;
    }

    QDialog dlg(advWin_ ? advWin_ : static_cast<QWidget*>(mainWin_));
    dlg.setWindowTitle(QStringLiteral("Salsa Config — ") + cfgPath);
    dlg.setMinimumSize(680, 520);
    auto* lay = new QVBoxLayout(&dlg);
    auto* pathLbl = new QLabel(QStringLiteral("Config File: %1").arg(cfgPath), &dlg);
    pathLbl->setWordWrap(true);
    pathLbl->setObjectName(QStringLiteral("DiagStatusLabel"));
    lay->addWidget(pathLbl);

    auto* txt = new QPlainTextEdit(&dlg);
    txt->setReadOnly(true);
    txt->setPlainText(QString::fromUtf8(QJsonDocument(loadGuiSettings()).toJson(QJsonDocument::Indented)));
    lay->addWidget(txt, 1);

    auto* btnRow = new QHBoxLayout();
    auto* btnDir = new QPushButton(QStringLiteral("Open Folder"), &dlg);
    connect(btnDir, &QPushButton::clicked, &dlg, [&fi]() {
        QDesktopServices::openUrl(QUrl::fromLocalFile(fi.absolutePath()));
    });
    btnRow->addWidget(btnDir);
    btnRow->addStretch(1);
    auto* btnClose = new QPushButton(QStringLiteral("Close"), &dlg);
    connect(btnClose, &QPushButton::clicked, &dlg, &QDialog::accept);
    btnRow->addWidget(btnClose);
    lay->addLayout(btnRow);
    dlg.setStyleSheet(buildSalsaStylesheet(currentTheme_, false, fontPct_));
    dlg.exec();
}

void StudioController::updateSpectrumFromState() {
    if (!spectrum_) return;
    std::array<float, SALSA_EQ_BANDS> gains{}, qs{}, freqs{}, deltas{}, levels{};
    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
        gains[i] = eqCfg_.band_gains_db[i];
        qs[i] = eqCfg_.band_q[i];
        freqs[i] = paramExt_.band_freq_hz[i];
        deltas[i] = status_.eq_dyn_delta_db[i];
        levels[i] = status_.eq_band_levels_db[i];
    }
    spectrum_->updateTelemetry(
        eqCfg_.enabled != 0, eqCfg_.dynamic_mode, eqCfg_.dyn_threshold_db,
        gains, qs, freqs, deltas, levels);
}

void StudioController::updateBandParamTable() {
    if (!tblParamBands_) return;
    updatingUi_ = true;
    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
        if (auto* spnHz = qobject_cast<QDoubleSpinBox*>(tblParamBands_->cellWidget(i, 1))) {
            spnHz->setValue(paramExt_.band_freq_hz[i]);
        }
        if (auto* spnQ = qobject_cast<QDoubleSpinBox*>(tblParamBands_->cellWidget(i, 2))) {
            spnQ->setValue(eqCfg_.band_q[i]);
        }
        if (auto* cmbT = qobject_cast<QComboBox*>(tblParamBands_->cellWidget(i, 3))) {
            if (int idx = cmbT->findData(static_cast<int>(paramExt_.band_type[i])); idx >= 0) cmbT->setCurrentIndex(idx);
        }
        if (bandFreqLbls_[i]) bandFreqLbls_[i]->setText(fmtFreqHz(paramExt_.band_freq_hz[i]));
    }
    updatingUi_ = false;
}

void StudioController::onPresetActivated(int index) {
    if (index < 0 || !cmbPreset_) return;
    const QString tag = cmbPreset_->itemData(index).toString();
    if (tag.startsWith(QLatin1String("builtin:"))) {
        const QString name = tag.mid(8);
        if (salsa_apply_builtin_eq_preset(name.toUtf8().constData(), &eqCfg_, &paramExt_) == 0) {
            updatingUi_ = true;
            if (int di = cmbDynMode_->findData(static_cast<int>(eqCfg_.dynamic_mode)); di >= 0) cmbDynMode_->setCurrentIndex(di);
            lblSavedDynMode_->setText(dynModeSavedLabel(eqCfg_.dynamic_mode));
            sldPreamp_->setValue(static_cast<int>(std::lround(eqCfg_.preamp_db * 10.0)));
            sldThresh_->setValue(static_cast<int>(std::lround(eqCfg_.dyn_threshold_db * 10.0)));
            sldRatio_->setValue(static_cast<int>(std::lround(eqCfg_.dyn_ratio * 10.0)));
            for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
                bandSliders_[i]->setValue(static_cast<int>(std::lround(eqCfg_.band_gains_db[i] * 10.0)));
                bandValLbls_[i]->setText(QStringLiteral("%1%2 dB").arg(eqCfg_.band_gains_db[i] >= 0 ? QStringLiteral("+") : QString()).arg(eqCfg_.band_gains_db[i], 0, 'f', 1));
            }
            lblSavedPreset_->setText(QStringLiteral("Saved: %1").arg(name));
            updateBandParamTable();
            updatingUi_ = false;
            pushEqToEngine();
        }
    } else if (tag.startsWith(QLatin1String("autoeq:"))) {
        const QString name = tag.mid(7);
        for (const QJsonValue& av : savedAutoEq_) {
            const QJsonObject ao = av.toObject();
            if (ao.value(QStringLiteral("name")).toString() == name) {
                const QByteArray pjs = jsonCompact(ao).toUtf8();
                if (salsa_apply_saved_autoeq(pjs.constData(), &eqCfg_, &paramExt_) == 0) {
                    updatingUi_ = true;
                    chkEqEnable_->setChecked(true);
                    sldPreamp_->setValue(static_cast<int>(std::lround(eqCfg_.preamp_db * 10.0)));
                    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
                        bandSliders_[i]->setValue(static_cast<int>(std::lround(eqCfg_.band_gains_db[i] * 10.0)));
                        bandValLbls_[i]->setText(QStringLiteral("%1%2 dB").arg(eqCfg_.band_gains_db[i] >= 0 ? QStringLiteral("+") : QString()).arg(eqCfg_.band_gains_db[i], 0, 'f', 1));
                    }
                    lblSavedPreset_->setText(QStringLiteral("Saved: %1").arg(name));
                    updateBandParamTable();
                    updatingUi_ = false;
                    pushEqToEngine();
                }
                break;
            }
        }
    }
}

void StudioController::onResetFlatClicked() {
    updatingUi_ = true;
    salsa_apply_builtin_eq_preset("Flat / Studio Reference", &eqCfg_, &paramExt_);
    paramExt_.crossfeed_enabled = 0;
    chkCrossfeed_->setChecked(false);
    sldPreamp_->setValue(0);
    sldThresh_->setValue(-180);
    sldRatio_->setValue(25);
    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
        bandSliders_[i]->setValue(0);
        bandValLbls_[i]->setText(QStringLiteral("+0.0 dB"));
    }
    lblSavedPreset_->setText(QStringLiteral("Saved: Flat / Studio Reference"));
    lblSavedDynMode_->setText(dynModeSavedLabel(eqCfg_.dynamic_mode));
    populatePresetDropdown(QStringLiteral("Flat / Studio Reference"));
    updateBandParamTable();
    updatingUi_ = false;
    pushEqToEngine();
}

void StudioController::onImportAutoEqClicked() {
    const QString path = QFileDialog::getOpenFileName(
        mainWin_, QStringLiteral("Import AutoEQ / EqualizerAPO Parametric .txt"),
        QDir::homePath(), QStringLiteral("EqualizerAPO Profiles (*.txt);;All Files (*)"));
    if (path.isEmpty()) return;

    const QByteArray pBytes = path.toUtf8();
    const QByteArray listBytes = jsonCompact(savedAutoEq_).toUtf8();
    const QJsonObject res = parseJsonObj(SalsaStr(salsa_import_autoeq_txt_json(pBytes.constData(), listBytes.constData())).toQString());
    if (!res.value(QStringLiteral("ok")).toBool(false)) {
        QMessageBox::warning(mainWin_, QStringLiteral("AutoEQ Import Failed"), res.value(QStringLiteral("error")).toString());
        return;
    }
    const QJsonObject preset = res.value(QStringLiteral("preset")).toObject();
    const QString name = preset.value(QStringLiteral("name")).toString();

    // Replace existing entry only if same source_path/name; never erase other saved profiles
    QJsonArray updated;
    bool replaced = false;
    for (const QJsonValue& v : savedAutoEq_) {
        const QJsonObject cur = v.toObject();
        if (cur.value(QStringLiteral("name")).toString() == name) {
            updated.append(preset);
            replaced = true;
        } else {
            updated.append(cur);
        }
    }
    if (!replaced) updated.append(preset);
    savedAutoEq_ = updated;
    salsa_save_autoeq_list_json(jsonCompact(savedAutoEq_).toUtf8().constData());

    populatePresetDropdown(name);
    const int idx = cmbPreset_->findData(QStringLiteral("autoeq:") + name);
    if (idx >= 0) {
        cmbPreset_->setCurrentIndex(idx);
        onPresetActivated(idx);
    }
}

void StudioController::onReloadAutoEqClicked() {
    QJsonObject target;
    const QString curName = cFixedStr(eqCfg_.preset_name, sizeof(eqCfg_.preset_name));
    for (const QJsonValue& v : savedAutoEq_) {
        const QJsonObject ao = v.toObject();
        if (ao.value(QStringLiteral("name")).toString() == curName) {
            target = ao;
            break;
        }
    }
    if (target.isEmpty() && !savedAutoEq_.isEmpty()) {
        // Also check current dropdown selection if user highlighted one
        const QString tag = cmbPreset_->currentData().toString();
        if (tag.startsWith(QLatin1String("autoeq:"))) {
            const QString nm = tag.mid(7);
            for (const QJsonValue& v : savedAutoEq_) {
                if (v.toObject().value(QStringLiteral("name")).toString() == nm) {
                    target = v.toObject();
                    break;
                }
            }
        }
    }
    if (target.isEmpty()) {
        QMessageBox::information(mainWin_, QStringLiteral("Reload AutoEQ .txt"),
                                 QStringLiteral("Select or apply an imported AutoEQ profile first, or click 'Import .txt' to load one from disk."));
        return;
    }

    const QJsonObject res = parseJsonObj(
        SalsaStr(salsa_reload_autoeq_preset_json(jsonCompact(target).toUtf8().constData())).toQString());
    if (!res.value(QStringLiteral("ok")).toBool(false)) {
        QMessageBox::warning(mainWin_, QStringLiteral("Reload Failed"), res.value(QStringLiteral("error")).toString());
        return;
    }
    const QJsonObject reloaded = res.value(QStringLiteral("preset")).toObject();
    const QString name = reloaded.value(QStringLiteral("name")).toString();

    QJsonArray updated;
    for (const QJsonValue& v : savedAutoEq_) {
        const QJsonObject cur = v.toObject();
        updated.append(cur.value(QStringLiteral("name")).toString() == name ? reloaded : cur);
    }
    savedAutoEq_ = updated;
    salsa_save_autoeq_list_json(jsonCompact(savedAutoEq_).toUtf8().constData());

    populatePresetDropdown(name);
    const int idx = cmbPreset_->findData(QStringLiteral("autoeq:") + name);
    if (idx >= 0) {
        cmbPreset_->setCurrentIndex(idx);
        onPresetActivated(idx);
    }
}

void StudioController::onDynamicsSliderChanged() {
    const double pre = sldPreamp_->value() / 10.0;
    const double thr = sldThresh_->value() / 10.0;
    const double rat = sldRatio_->value() / 10.0;
    lblPreamp_->setText(QStringLiteral("Preamp: %1%2 dB").arg(pre >= 0 ? QStringLiteral("+") : QString()).arg(pre, 0, 'f', 1));
    lblThresh_->setText(QStringLiteral("Dyn Threshold: %1 dB").arg(thr, 0, 'f', 1));
    lblRatio_->setText(QStringLiteral("Dyn Ratio: %1:1").arg(rat, 0, 'f', 1));
    if (!updatingUi_) pushEqToEngine();
}

void StudioController::onBandSliderChanged(int band, int tenths) {
    const double db = tenths / 10.0;
    bandValLbls_[band]->setText(QStringLiteral("%1%2 dB").arg(db >= 0 ? QStringLiteral("+") : QString()).arg(db, 0, 'f', 1));
    if (!updatingUi_) {
        eqCfg_.band_gains_db[band] = static_cast<float>(db);
        salsa_set_eq_preset_name(&eqCfg_, "Custom");
        lblSavedPreset_->setText(QStringLiteral("Saved: Custom"));
        pushEqToEngine();
    }
}

void StudioController::savePerDeviceEqForSink(const QString& sinkId) {
    if (sinkId.isEmpty()) return;
    QJsonObject map = guiSettings_.value(QStringLiteral("per_device_eq")).toObject();
    map.insert(sinkId, parseJsonObj(SalsaStr(salsa_eq_cfg_to_json(&eqCfg_, &paramExt_)).toQString()));
    guiSettings_.insert(QStringLiteral("per_device_eq"), map);
    updateGuiSettingsPatch(QJsonObject{{QStringLiteral("per_device_eq"), map}});
}

bool StudioController::loadPerDeviceEqForSink(const QString& sinkId) {
    if (sinkId.isEmpty()) return false;
    const QJsonObject map = guiSettings_.value(QStringLiteral("per_device_eq")).toObject();
    if (!map.contains(sinkId)) return false;
    const QByteArray js = jsonCompact(map.value(sinkId).toObject()).toUtf8();
    if (salsa_json_to_eq_cfg(js.constData(), &eqCfg_, &paramExt_) != 0) return false;

    updatingUi_ = true;
    chkEqEnable_->setChecked(eqCfg_.enabled != 0);
    if (int di = cmbDynMode_->findData(static_cast<int>(eqCfg_.dynamic_mode)); di >= 0) cmbDynMode_->setCurrentIndex(di);
    lblSavedDynMode_->setText(dynModeSavedLabel(eqCfg_.dynamic_mode));
    sldPreamp_->setValue(static_cast<int>(std::lround(eqCfg_.preamp_db * 10.0)));
    sldThresh_->setValue(static_cast<int>(std::lround(eqCfg_.dyn_threshold_db * 10.0)));
    sldRatio_->setValue(static_cast<int>(std::lround(eqCfg_.dyn_ratio * 10.0)));
    chkLimiter_->setChecked(eqCfg_.limiter_enabled != 0);
    chkCrossfeed_->setChecked(paramExt_.crossfeed_enabled != 0);
    sldCrossfeed_->setValue(static_cast<int>(std::lround(paramExt_.crossfeed_level * 100.0)));
    if (int ci = cmbCrossfeedCut_->findData(static_cast<int>(paramExt_.crossfeed_cutoff_hz)); ci >= 0) cmbCrossfeedCut_->setCurrentIndex(ci);
    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
        bandSliders_[i]->setValue(static_cast<int>(std::lround(eqCfg_.band_gains_db[i] * 10.0)));
        bandValLbls_[i]->setText(QStringLiteral("%1%2 dB").arg(eqCfg_.band_gains_db[i] >= 0 ? QStringLiteral("+") : QString()).arg(eqCfg_.band_gains_db[i], 0, 'f', 1));
    }
    const QString pn = cFixedStr(eqCfg_.preset_name, sizeof(eqCfg_.preset_name));
    lblSavedPreset_->setText(QStringLiteral("Saved: %1").arg(pn));
    populatePresetDropdown(pn);
    updateBandParamTable();
    updatingUi_ = false;
    pushEqToEngine();
    return true;
}

void StudioController::onPerDeviceEqToggled(bool on) {
    guiSettings_.insert(QStringLiteral("eq_per_device"), on);
    updateGuiSettingsPatch(QJsonObject{{QStringLiteral("eq_per_device"), on}});
    if (on && !lastSinkId_.isEmpty()) savePerDeviceEqForSink(lastSinkId_);
}

void StudioController::onClockChanged() {
    if (updatingUi_) return;
    const uint32_t rate = static_cast<uint32_t>(qvarInt(cmbRate_->currentData(), 48000));
    const uint32_t period = static_cast<uint32_t>(qvarInt(cmbPeriod_->currentData(), 256));
    const uint32_t bits = static_cast<uint32_t>(qvarInt(cmbBits_->currentData(), 32));
    salsa_set_engine_config(rate, period, bits);
    saveStudioSettings();
}

void StudioController::onCapRateChanged() {
    if (updatingUi_) return;
    const uint32_t rate = static_cast<uint32_t>(qvarInt(cmbCapRate_->currentData(), 0));
    salsa_set_capture_sample_rate(rate);
    saveStudioSettings();
}

void StudioController::onSinkActivated(int idx) {
    const QString targetId = cmbSink_->itemData(idx).toString();
    targetPbId_ = targetId.isEmpty() ? QStringLiteral("auto") : targetId;
    if (chkEqPerDev_->isChecked() && !lastSinkId_.isEmpty()) savePerDeviceEqForSink(lastSinkId_);
    salsa_set_devices(targetId.toUtf8().constData(), nullptr);
    lastSinkId_ = targetId;
    if (chkEqPerDev_->isChecked()) loadPerDeviceEqForSink(targetId);
}

void StudioController::onSourceActivated(int idx) {
    const QString targetId = cmbSource_->itemData(idx).toString();
    targetCapId_ = targetId.isEmpty() ? QStringLiteral("auto") : targetId;
    salsa_set_devices(nullptr, targetId.toUtf8().constData());
}

void StudioController::onBtResetClicked() {
    btnBtReset_->setEnabled(false);
    const SalsaStr msg(salsa_bt_reset_sync());
    btnBtReset_->setEnabled(true);
    lblBtDiag_->setText(msg.toQString());
}

void StudioController::onBtCfgChanged() {
    btHoldUntilMs_ = nowMs() + 650;
    btCfg_.preferred_codec = static_cast<uint32_t>(cmbBtCodec_->currentData().toInt());
    btCfg_.ldac_mode = static_cast<uint32_t>(cmbBtLdac_->currentData().toInt());
    btCfg_.wireless_abr_enabled = chkBtAbr_->isChecked() ? 1 : 0;
    btCfg_.hfp_sco_enabled = chkBtSco_->isChecked() ? 1 : 0;
    salsa_set_bt_codec_config(&btCfg_);
}

void StudioController::scheduleHwAnalogSend(bool immediate) {
    hwHoldUntilMs_ = nowMs() + 1200;
    if (!hwSendTimer_ || immediate) {
        onHwAnalogCfgChanged();
        return;
    }
    if (!hwSendTimer_->isActive()) {
        hwSendTimer_->start(40);
    }
}

void StudioController::onHwAnalogCfgChanged() {
    if (hwSendTimer_ && hwSendTimer_->isActive()) {
        hwSendTimer_->stop();
    }
    hwHoldUntilMs_ = nowMs() + 1200;
    hwAnalog_.hw_dac_unity_vol = chkHwDacUnity_->isChecked() ? 1 : 0;
    hwAnalog_.hp_amp_tier = static_cast<uint32_t>(cmbHwAmp_->currentData().toInt());
    hwAnalog_.output_route = static_cast<uint32_t>(cmbHwRoute_->currentData().toInt());
    hwAnalog_.unlock_0db_rails = chkHwRails_->isChecked() ? 1 : 0;
    hwAnalog_.analog_boost_db = sldHwBoost_->value() * 0.5f;
    hwAnalog_.mic_hw_boost_db = static_cast<float>(sldHwMicBoost_->value());
    salsa_set_hw_analog(&hwAnalog_);
    hwHoldUntilMs_ = nowMs() + 1200;
}

void StudioController::onHwAnalogReset() {
    updatingUi_ = true;
    chkHwDacUnity_->setChecked(true);
    cmbHwAmp_->setCurrentIndex(0);
    cmbHwRoute_->setCurrentIndex(0);
    chkHwRails_->setChecked(false);
    sldHwBoost_->setValue(0);
    lblHwBoost_->setText(formatBoostDbText(0.0));
    sldHwMicBoost_->setValue(0);
    lblHwMicBoost_->setText(formatBoostDbText(0.0));
    updatingUi_ = false;
    onHwAnalogCfgChanged();
}

void StudioController::onVolStepChanged() {
    volStepPct_ = normalizeVolStepPct(qvarInt(cmbVolStep_->currentData(), 5));
    updateGuiSettingsPatch(QJsonObject{{QStringLiteral("volume_step_pct"), volStepPct_}});
}

void StudioController::sendSpkVolNow(int muteOverride) {
    if (spkSendTimer_ && spkSendTimer_->isActive()) spkSendTimer_->stop();
    const int muted = (muteOverride >= 0) ? (muteOverride != 0 ? 1 : 0) : (status_.desktop_muted ? 1 : 0);
    spkHoldUntilMs_ = nowMs() + 800;
    salsa_set_master_volume(sldSpk_->value(), muted);
    spkHoldUntilMs_ = nowMs() + 800;
}

void StudioController::sendMicVolNow(int muteOverride) {
    if (micSendTimer_ && micSendTimer_->isActive()) micSendTimer_->stop();
    const int muted = (muteOverride >= 0) ? (muteOverride != 0 ? 1 : 0) : (status_.desktop_mic_muted ? 1 : 0);
    micHoldUntilMs_ = nowMs() + 800;
    salsa_set_mic_volume(sldMic_->value(), muted);
    micHoldUntilMs_ = nowMs() + 800;
}

void StudioController::onSpkSliderMoved(int v) {
    lblSpk_->setText(QStringLiteral("Speaker Vol (%1%):").arg(v));
    spkHoldUntilMs_ = nowMs() + 800;
    if (!spkSendTimer_->isActive()) spkSendTimer_->start(40);
}

void StudioController::onMicSliderMoved(int v) {
    lblMic_->setText(QStringLiteral("Mic Capture (%1%):").arg(v));
    micHoldUntilMs_ = nowMs() + 800;
    if (!micSendTimer_->isActive()) micSendTimer_->start(40);
}

void StudioController::onSessionSliderMoved(uint32_t sid, int v) {
    const qint64 now = nowMs();
    for (auto& rw : sessRows_) {
        if (rw.sid != sid) continue;
        rw.holdUntilMs = now + 450;
        if (now - rw.lastSentMs >= 40) {
            rw.lastSentMs = now;
            salsa_set_session_control(sid, v, -1, nullptr);
        }
        break;
    }
}

void StudioController::flushSessionVolume(uint32_t sid, int v) {
    const qint64 now = nowMs();
    for (auto& rw : sessRows_) {
        if (rw.sid != sid) continue;
        rw.holdUntilMs = now + 450;
        rw.lastSentMs = now;
        break;
    }
    salsa_set_session_control(sid, v, -1, nullptr);
}

void StudioController::syncGraphTimer() {
    if (!graphAnimTimer_) return;
    if (introWin_ && introWin_->isVisible()) {
        if (!graphAnimTimer_->isActive()) graphAnimTimer_->start();
        if (dbgSnapTimer_ && !dbgSnapTimer_->isActive()) dbgSnapTimer_->start();
    } else {
        graphAnimTimer_->stop();
        if (dbgSnapTimer_) dbgSnapTimer_->stop();
    }
}

void StudioController::openTopologyWindow() {
    if (!studioDbgHandle_) studioDbgHandle_ = salsa_dbg_create();
    raiseAndActivateWindow(introWin_);
    syncGraphTimer();
    pollEngine(false);
}

void StudioController::enterStudioFromTopology() {
    studioEntered_ = true;
    raiseAndActivateWindow(mainWin_);
    if (introWin_) introWin_->hide();
    syncGraphTimer();
    pollEngine(false);
}

void StudioController::openAdvancedWindow() {
    refreshAdvIntegration();
    raiseAndActivateWindow(advWin_);
}

void StudioController::openAboutWindow(bool standaloneQuit) {
    QWidget* parent = nullptr;
    if (mainWin_ && mainWin_->isVisible()) parent = mainWin_;
    else if (introWin_ && introWin_->isVisible()) parent = introWin_;
    else if (advWin_ && advWin_->isVisible()) parent = advWin_;
    showAboutLicenseDialog(parent, QStringLiteral("Salsa Studio"), currentTheme_, fontPct_);
    if (standaloneQuit
        && !closingAll_
        && (!mainWin_ || !mainWin_->isVisible())
        && (!introWin_ || !introWin_->isVisible())
        && (!advWin_ || !advWin_->isVisible())) {
        closingAll_ = true;
        QApplication::quit();
    }
}

void StudioController::selectManualChapter(const QString& key) {
    if (key.isEmpty()) return;
    currentManualKey_ = key;
    if (graphCanvas_) graphCanvas_->setSelectedNode(key);
    for (auto it = manualBtns_.constBegin(); it != manualBtns_.constEnd(); ++it) {
        it.value()->setObjectName(it.key() == key ? QStringLiteral("AccentBtn") : QStringLiteral("CompactBtn"));
        it.value()->style()->unpolish(it.value());
        it.value()->style()->polish(it.value());
    }
    if (manualBrowser_) {
        manualBrowser_->document()->setDefaultStyleSheet(
            QStringLiteral("body, p, li { color: %1; } code { background-color: %2; color: %3; padding: 1px 4px; border-radius: 3px; }")
                .arg(currentTheme_.color("text_primary", "#e2e8f0"),
                     currentTheme_.color("code_bg", "#232830"),
                     currentTheme_.color("accent_secondary", "#38bdf8")));
        const QByteArray themeJs = jsonCompact(themeToJsonObj(currentTheme_)).toUtf8();
        const SalsaStr html(salsa_render_manual_chapter_html(key.toUtf8().constData(), themeJs.constData()));
        manualBrowser_->setHtml(html.toQString());
    }
}

void StudioController::openLiveDebuggerWindow() {
    QWidget* src = QApplication::activeWindow();
    if (!src) src = (advWin_ && advWin_->isVisible()) ? static_cast<QWidget*>(advWin_) : static_cast<QWidget*>(mainWin_);
    spawnDetachedWaylandBinary(QStringLiteral("salsa-debug"), {}, src);
}

void StudioController::runSystemdRestart(const QString& unit, QPushButton* btn) {
    if (!btn) return;
    const QString orig = btn->text();
    btn->setEnabled(false);
    btn->setText(QStringLiteral("Restarting..."));
    QApplication::processEvents();
    salsa_ctl_systemd(unit.toUtf8().constData(), "restart");
    btn->setEnabled(true);
    btn->setText(orig);
    lastDevSig_.clear();
    QTimer::singleShot(300, this, [this]() { pollEngine(true); });
}

void StudioController::refreshAdvIntegration() {
    const QJsonObject st = parseJsonObj(SalsaStr(salsa_get_integration_status_json()).toQString());
    auto styleChip = [this](QLabel* lbl, const QString& txt, bool ok) {
        lbl->setText(txt);
        const QString col = ok ? currentTheme_.color("accent_primary", "#00e676")
                               : currentTheme_.color("accent_warn", "#f59e0b");
        lbl->setStyleSheet(QStringLiteral("background-color: %1; color: %2; border: 1px solid %3; border-radius: 5px; padding: 2px 7px; font-weight: 700;")
                               .arg(currentTheme_.color("bg_deep"), col, col));
    };
    const bool steam = st.value(QStringLiteral("steam_ready")).toBool(false);
    const bool lnch = st.value(QStringLiteral("launchers_ok")).toBool(false);
    const bool pac = st.value(QStringLiteral("pacman_hook")).toBool(false);
    const bool watch = st.value(QStringLiteral("watcher_enabled")).toBool(false);
    const bool eng = st.value(QStringLiteral("engine_active")).toBool(true);

    styleChip(lblChipSteam_, steam ? QStringLiteral("● Steam: Ready") : QStringLiteral("● Steam: Missing"), steam);
    styleChip(lblChipLaunchers_, lnch ? QStringLiteral("● Launchers: OK") : QStringLiteral("● Launchers: Sync"), lnch);
    styleChip(lblChipPacman_, pac ? QStringLiteral("● Pacman Hook: ON") : QStringLiteral("● Pacman Hook: OFF"), pac);

    chkAutosyncNow_->blockSignals(true);
    chkAutosyncNow_->setChecked(watch);
    chkAutosyncNow_->blockSignals(false);
    btnTogglePacman_->setChecked(pac);
    btnTogglePacman_->setText(
        pac ? QStringLiteral("Remove Pacman Hook (pkexec)")
            : QStringLiteral("Install Pacman Hook (pkexec)"));
    btnEngineToggle_->setText(
        eng ? QStringLiteral("Engine: SALSA NATIVE AUDIO (Revert to PW/WP)")
            : QStringLiteral("Engine: PIPEWIRE/WP FALLBACK (Restore Salsa Native)"));
}

void StudioController::checkSettingsFileSync() {
    const uint64_t mtime = salsa_gui_settings_mtime_ns();
    if (mtime != 0 && mtime != lastSettingsMtimeNs_) {
        lastSettingsMtimeNs_ = mtime;
        guiSettings_ = loadGuiSettings();
        const int diskPct = normalizeFontScalePct(salsa_load_font_scale());
        if (diskPct != fontPct_) {
            fontPct_ = diskPct;
            syncFontScaleBars();
            applyThemeAndFontAll(false);
        }
        const int diskStep = normalizeVolStepPct(guiSettings_.value(QStringLiteral("volume_step_pct")).toInt(volStepPct_));
        if (diskStep != volStepPct_) {
            volStepPct_ = diskStep;
            if (int si = cmbVolStep_->findData(volStepPct_); si >= 0) {
                cmbVolStep_->blockSignals(true);
                cmbVolStep_->setCurrentIndex(si);
                cmbVolStep_->blockSignals(false);
            }
        }
        if (!themeDlg_ || !themeDlg_->isVisible()) {
            const SalsaThemeData diskTheme = loadThemeForApp(false);
            const QString diskThemeName = !diskTheme.name.isEmpty()
                ? diskTheme.name
                : guiSettings_.value(QStringLiteral("theme")).toString(currentThemeName_);
            if (diskThemeName != currentThemeName_ ||
                diskTheme.isLight != currentTheme_.isLight ||
                diskTheme.colors != currentTheme_.colors) {
                currentTheme_ = diskTheme;
                currentThemeName_ = diskThemeName;
                if (cmbTheme_) {
                    cmbTheme_->blockSignals(true);
                    cmbTheme_->setCurrentText(currentThemeName_);
                    cmbTheme_->blockSignals(false);
                }
                applyThemeAndFontAll(false);
            }
        }
    }
}

void StudioController::pollEngine(bool initial) {
    const qint64 pollStartMs = nowMs();
    SalsaEngineStatus st{};
    const bool gotSt = (salsa_get_engine_status(&st) == 0);
    const qint64 tNow = nowMs();
    SalsaDeviceList dl{};
    bool gotDl = false;
    if (initial || tNow - lastDevPollMs_ >= 600) {
        lastDevPollMs_ = tNow;
        gotDl = (salsa_list_devices(&dl) == 0);
    }

    if (gotDl) {
        devices_ = dl;
        QString sig;
        for (uint32_t i = 0; i < std::min<uint32_t>(dl.num_devices, SALSA_MAX_DEVICES); ++i) {
            sig += cFixedStr(dl.devices[i].id, sizeof(dl.devices[i].id)) + QChar(',')
                   + cFixedStr(dl.devices[i].name, sizeof(dl.devices[i].name)) + QChar('|');
        }
        if (sig != lastDevSig_ && !cmbSink_->view()->isVisible() && !cmbSource_->view()->isVisible()) {
            lastDevSig_ = sig;
            updatingUi_ = true;
            cmbSink_->clear();
            cmbSink_->addItem(QStringLiteral("Auto-Priority (Dual USB > BT A2DP > USB > Analog > HDMI)"), QStringLiteral("auto"));
            cmbSource_->clear();
            cmbSource_->addItem(QStringLiteral("Auto-Priority (USB Mic > HDA > Virtual)"), QStringLiteral("auto"));
            for (uint32_t i = 0; i < std::min<uint32_t>(dl.num_devices, SALSA_MAX_DEVICES); ++i) {
                const auto& d = dl.devices[i];
                const QString did = cFixedStr(d.id, sizeof(d.id));
                const QString dname = cFixedStr(d.name, sizeof(d.name));
                const QString disp = dname.isEmpty() ? did : dname;
                if (d.is_capture) cmbSource_->addItem(disp, did);
                else cmbSink_->addItem(disp, did);
            }
            updatingUi_ = false;
        }
    }

    if (!gotSt) {
        lblEndpoints_->setText(QStringLiteral("Audio engine offline — start lwasapi-audiodg.service to manage live streams"));
        if (pollTimer_ && pollTimer_->interval() != 450) pollTimer_->setInterval(450);
        return;
    }

    hasStatus_ = true;
    status_ = st;

    const QString actPbId = cFixedStr(st.active_playback_id, sizeof(st.active_playback_id));
    const QString actCapId = cFixedStr(st.active_capture_id, sizeof(st.active_capture_id));
    const QString hwDev = friendlyDeviceLabel(
        devices_,
        actPbId,
        cFixedStr(st.hw_device, sizeof(st.hw_device)),
        false);
    const QString hwCap = friendlyDeviceLabel(
        devices_,
        actCapId,
        cFixedStr(st.hw_capture_device, sizeof(st.hw_capture_device)),
        true);

    if (!actPbId.isEmpty() && actPbId != lastSinkId_) {
        lastSinkId_ = actPbId;
        if (!initial && chkEqPerDev_->isChecked()) loadPerDeviceEqForSink(actPbId);
    }

    updatingUi_ = true;
    const double latMs = st.hw_sample_rate > 0 ? (static_cast<double>(st.hw_period_frames) * 1000.0 / st.hw_sample_rate) : 5.33;
    lblClockSrc_->setText(QStringLiteral("Master Clock: %1 kHz / %2-bit PCM").arg(st.hw_sample_rate / 1000.0, 0, 'g', 4).arg(st.hw_format_bits));
    lblLatency_->setText(QStringLiteral("Quantum: %1 ms | SCHED_FIFO Prio %2").arg(latMs, 0, 'f', 2).arg(st.realtime_priority));

    if (initial) {
        if (int ri = cmbRate_->findData(static_cast<int>(st.hw_sample_rate)); ri >= 0) cmbRate_->setCurrentIndex(ri);
        if (int pi = cmbPeriod_->findData(static_cast<int>(st.hw_period_frames)); pi >= 0) cmbPeriod_->setCurrentIndex(pi);
        if (int bi = cmbBits_->findData(static_cast<int>(st.hw_format_bits)); bi >= 0) cmbBits_->setCurrentIndex(bi);
    }
    if (int si = cmbSink_->findData(actPbId); si >= 0) cmbSink_->setCurrentIndex(si);
    if (int ci = cmbSource_->findData(actCapId); ci >= 0) cmbSource_->setCurrentIndex(ci);

    if (!sldSpk_->isSliderDown()
        && !(spkSendTimer_ && spkSendTimer_->isActive())
        && pollStartMs >= spkHoldUntilMs_
        && tNow >= spkHoldUntilMs_) {
        sldSpk_->setValue(static_cast<int>(st.desktop_volume_pct));
        lblSpk_->setText(QStringLiteral("Speaker Vol (%1%):").arg(st.desktop_volume_pct));
        btnSpkMute_->setText(st.desktop_muted ? QStringLiteral("Unmute Spk") : QStringLiteral("Mute Spk"));
    }
    if (!sldMic_->isSliderDown()
        && !(micSendTimer_ && micSendTimer_->isActive())
        && pollStartMs >= micHoldUntilMs_
        && tNow >= micHoldUntilMs_) {
        sldMic_->setValue(static_cast<int>(st.desktop_mic_volume_pct));
        lblMic_->setText(QStringLiteral("Mic Capture (%1%):").arg(st.desktop_mic_volume_pct));
        btnMicMute_->setText(st.desktop_mic_muted ? QStringLiteral("Unmute Mic") : QStringLiteral("Mute Mic"));
    }

    if (tNow >= eqHoldUntilMs_) {
        if (initial) {
            eqCfg_ = st.eq_cfg;
            paramExt_ = st.param_eq;
            chkEqEnable_->setChecked(eqCfg_.enabled != 0);
            if (int di = cmbDynMode_->findData(static_cast<int>(eqCfg_.dynamic_mode)); di >= 0) cmbDynMode_->setCurrentIndex(di);
            lblSavedDynMode_->setText(dynModeSavedLabel(eqCfg_.dynamic_mode));
            chkLimiter_->setChecked(eqCfg_.limiter_enabled != 0);
            chkCrossfeed_->setChecked(paramExt_.crossfeed_enabled != 0);
            sldCrossfeed_->setValue(static_cast<int>(std::lround(paramExt_.crossfeed_level * 100.0)));
            lblCrossfeed_->setText(formatPctText(sldCrossfeed_->value()));
            if (int ci = cmbCrossfeedCut_->findData(static_cast<int>(paramExt_.crossfeed_cutoff_hz)); ci >= 0) cmbCrossfeedCut_->setCurrentIndex(ci);
            sldPreamp_->setValue(static_cast<int>(std::lround(eqCfg_.preamp_db * 10.0)));
            sldThresh_->setValue(static_cast<int>(std::lround(eqCfg_.dyn_threshold_db * 10.0)));
            sldRatio_->setValue(static_cast<int>(std::lround(eqCfg_.dyn_ratio * 10.0)));
            for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
                bandSliders_[i]->setValue(static_cast<int>(std::lround(eqCfg_.band_gains_db[i] * 10.0)));
                bandValLbls_[i]->setText(QStringLiteral("%1%2 dB").arg(eqCfg_.band_gains_db[i] >= 0 ? QStringLiteral("+") : QString()).arg(eqCfg_.band_gains_db[i], 0, 'f', 1));
            }
            const QString pn = cFixedStr(eqCfg_.preset_name, sizeof(eqCfg_.preset_name));
            lblSavedPreset_->setText(QStringLiteral("Saved: %1").arg(pn));
            populatePresetDropdown(pn);
        }
    }

    for (int i = 0; i < SALSA_EQ_BANDS; ++i) {
        const float d = st.eq_dyn_delta_db[i];
        bandDynLbls_[i]->setText(QStringLiteral("Δ%1%2").arg(d >= 0 ? QStringLiteral("+") : QString()).arg(d, 0, 'f', 1));
    }
    const double pkL = std::clamp(st.output_peak_l > 1e-5f ? 20.0 * std::log10(st.output_peak_l) : -100.0, -100.0, 6.0);
    const double pkR = std::clamp(st.output_peak_r > 1e-5f ? 20.0 * std::log10(st.output_peak_r) : -100.0, -100.0, 6.0);
    const QString peakText = QStringLiteral("Output Peak L/R: %1 / %2 dBFS")
                                 .arg(pkL, 6, 'f', 1)
                                 .arg(pkR, 6, 'f', 1);
    if (peakText != lblPeaks_->text()) lblPeaks_->setText(peakText);

    if (tNow >= btHoldUntilMs_) {
        btCfg_ = st.bt_codec_cfg;
        if (int ci = cmbBtCodec_->findData(static_cast<int>(btCfg_.preferred_codec)); ci >= 0) cmbBtCodec_->setCurrentIndex(ci);
        if (int li = cmbBtLdac_->findData(static_cast<int>(btCfg_.ldac_mode)); li >= 0) cmbBtLdac_->setCurrentIndex(li);
        chkBtAbr_->setChecked(btCfg_.wireless_abr_enabled != 0);
        chkBtSco_->setChecked(btCfg_.hfp_sco_enabled != 0);
    }
    if (tNow >= btHoldUntilMs_ && (lastBtLinkPollMs_ == 0 || (tNow - lastBtLinkPollMs_) >= 1500)) {
        lastBtLinkPollMs_ = tNow;
        uint32_t btFlags = SALSA_BT_STAT_OPTIONAL;
        if (salsa_bt_connection_ctl(SALSA_BT_CONN_QUERY, "", &btFlags) == 0) {
            syncBtConnectionControls(btFlags);
        }
    }
    if (st.bt_active) {
        lblBtDiag_->setText(
            QStringLiteral("BT: %1 %2 kbps (%3 ms) %4")
                .arg(cFixedStr(st.bt_codec, sizeof(st.bt_codec)))
                .arg(st.bt_codec_cfg.effective_kbps)
                .arg(st.bt_delay_ms)
                .arg(cFixedStr(st.bt_codec_cfg.neg_reason, sizeof(st.bt_codec_cfg.neg_reason))));
    } else if (tNow >= btHoldUntilMs_) {
        const bool usbPri = (lastBtConnFlags_ & SALSA_BT_STAT_USB_MATCHES_BT) != 0;
        const bool conn = (lastBtConnFlags_ & SALSA_BT_STAT_CONNECTED) != 0;
        const bool opt = (lastBtConnFlags_ & SALSA_BT_STAT_OPTIONAL) != 0;
        if (usbPri) {
            lblBtDiag_->setText(conn
                ? QStringLiteral("BT: Standby (USB Priority Active)")
                : QStringLiteral("BT: Optional Off (USB Priority Active)"));
        } else {
            lblBtDiag_->setText(opt ? QStringLiteral("BT: Optional Standby") : QStringLiteral("BT: Standby"));
        }
    }

    const bool hwBusy = sldHwBoost_->isSliderDown()
        || sldHwMicBoost_->isSliderDown()
        || (hwSendTimer_ && hwSendTimer_->isActive())
        || (cmbHwAmp_->view() && cmbHwAmp_->view()->isVisible())
        || (cmbHwRoute_->view() && cmbHwRoute_->view()->isVisible());
    if (!hwBusy && pollStartMs >= hwHoldUntilMs_ && tNow >= hwHoldUntilMs_) {
        hwAnalog_ = st.hw_analog_cfg;
        chkHwDacUnity_->setChecked(hwAnalog_.hw_dac_unity_vol != 0);
        if (int ai = cmbHwAmp_->findData(static_cast<int>(hwAnalog_.hp_amp_tier)); ai >= 0) cmbHwAmp_->setCurrentIndex(ai);
        if (int ri = cmbHwRoute_->findData(static_cast<int>(hwAnalog_.output_route)); ri >= 0) cmbHwRoute_->setCurrentIndex(ri);
        chkHwRails_->setChecked(hwAnalog_.unlock_0db_rails != 0);
        sldHwBoost_->setValue(static_cast<int>(std::lround(hwAnalog_.analog_boost_db * 2.0f)));
        lblHwBoost_->setText(formatBoostDbText(hwAnalog_.analog_boost_db));
        sldHwMicBoost_->setValue(static_cast<int>(std::lround(hwAnalog_.mic_hw_boost_db)));
        lblHwMicBoost_->setText(formatBoostDbText(hwAnalog_.mic_hw_boost_db));
    }
    lblHwAnalogDiag_->setText(
        QStringLiteral("HW Stage: %1 | DAC: %2 | %3")
            .arg(cFixedStr(st.hw_analog_info.card_arch_name, sizeof(st.hw_analog_info.card_arch_name)),
                 cFixedStr(st.hw_analog_info.dac_ctl_name, sizeof(st.hw_analog_info.dac_ctl_name)),
                 cFixedStr(st.hw_analog_info.diag_note, sizeof(st.hw_analog_info.diag_note))));

    lblEndpoints_->setText(
        QStringLiteral("Sink: %1 | Mic: %2 | PA v35: %3 (%4 clients)")
            .arg(hwDev, hwCap,
                 st.pa_server_running ? QStringLiteral("Active") : QStringLiteral("Idle"))
            .arg(st.pa_clients));
    lblMidiStatus_->setText(
        QStringLiteral("DAW MIDI Hub: %1 (%2 events routed)")
            .arg(cFixedStr(st.midi_device, sizeof(st.midi_device)))
            .arg(st.midi_events_routed));
    updatingUi_ = false;

    updateSpectrumFromState();

    // Update Active Sessions table preserving existing widgets to prevent drag/focus interruption
    const uint32_t ns = std::min<uint32_t>(st.num_sessions, SALSA_MAX_SESSIONS);
    lblSessHdr_->setText(QStringLiteral("AUTO-RECOGNIZED FULL-DUPLEX SESSIONS (%1)").arg(ns));
    bool rebuildSess = (sessRows_.size() != ns);
    if (!rebuildSess) {
        for (uint32_t i = 0; i < ns; ++i) {
            if (sessRows_[i].sid != st.sessions[i].session_id) { rebuildSess = true; break; }
        }
    }
    if (rebuildSess) {
        sessRows_.clear();
        tblSessions_->setRowCount(static_cast<int>(ns));
        for (uint32_t i = 0; i < ns; ++i) {
            const auto& s = st.sessions[i];
            const uint32_t sid = s.session_id;
            const int row = static_cast<int>(i);
            auto mk = [](const QString& txt) {
                auto* it = new QTableWidgetItem(txt);
                it->setFlags(it->flags() & ~Qt::ItemIsEditable);
                return it;
            };
            tblSessions_->setItem(row, 0, mk(QString::number(sid)));
            tblSessions_->setItem(row, 1, mk(s.is_capture ? QStringLiteral("CAP") : QStringLiteral("PB")));
            tblSessions_->setItem(row, 2, mk(cFixedStr(s.client_name, sizeof(s.client_name))));
            tblSessions_->setItem(row, 3, mk(s.share_mode == 1 ? QStringLiteral("EXCL") : QStringLiteral("SHRD")));
            auto* fmtIt = mk(QStringLiteral("%1k/%2ch").arg(s.sample_rate / 1000.0, 0, 'g', 4).arg(s.channels));
            tblSessions_->setItem(row, 4, fmtIt);

            auto* sld = new StepVolumeSlider([this]() { return volStepPct_; }, Qt::Horizontal, tblSessions_);
            sld->setRange(0, 150);
            sld->setValue(static_cast<int>(s.volume_milli / 10));
            sld->setToolTip(QStringLiteral("Per-Session Volume — Scales %1 stream gain (0–150%, saved per application); scroll wheel steps cleanly by your configured increment.")
                                .arg(cFixedStr(s.client_name, sizeof(s.client_name))));
            connect(sld, &QSlider::valueChanged, this, [this, sid, sld](int v) {
                if (sld->signalsBlocked()) return;
                onSessionSliderMoved(sid, v);
            });
            connect(sld, &QSlider::sliderReleased, this, [this, sid, sld]() {
                flushSessionVolume(sid, sld->value());
            });
            tblSessions_->setCellWidget(row, 5, sld);

            auto* btnM = new QPushButton(s.muted ? QStringLiteral("Muted") : QStringLiteral("Mute"), tblSessions_);
            btnM->setObjectName(QStringLiteral("CompactBtn"));
            btnM->setCheckable(true);
            btnM->setChecked(s.muted != 0);
            btnM->setToolTip(QStringLiteral("Per-Session Mute — Mutes or unmutes this individual client inside the Salsa mixer without affecting other applications."));
            connect(btnM, &QPushButton::clicked, this, [sid, btnM]() {
                const bool m = btnM->isChecked();
                btnM->setText(m ? QStringLiteral("Muted") : QStringLiteral("Mute"));
                salsa_set_session_control(sid, -1, m ? 1 : 0, nullptr);
            });
            tblSessions_->setCellWidget(row, 6, btnM);

            auto* cmbR = new QComboBox(tblSessions_);
            cmbR->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
            cmbR->setMinimumContentsLength(12);
            cmbR->addItem(QStringLiteral("Default Sink/Mic"), QStringLiteral("auto"));
            cmbR->setToolTip(QStringLiteral("Per-Session Endpoint Route — Pins this client stream to a specific sink or capture device, or follows the active Default endpoint."));
            for (uint32_t d = 0; d < std::min<uint32_t>(devices_.num_devices, SALSA_MAX_DEVICES); ++d) {
                if ((devices_.devices[d].is_capture != 0) == (s.is_capture != 0)) {
                    cmbR->addItem(cFixedStr(devices_.devices[d].name, sizeof(devices_.devices[d].name)),
                                  cFixedStr(devices_.devices[d].id, sizeof(devices_.devices[d].id)));
                }
            }
            const QString tgt = cFixedStr(s.target_device, sizeof(s.target_device));
            if (int ri = cmbR->findData(tgt); ri >= 0) cmbR->setCurrentIndex(ri);
            connect(cmbR, &QComboBox::activated, this, [sid, cmbR](int idx) {
                const QString dId = cmbR->itemData(idx).toString();
                salsa_set_session_control(sid, -1, -1, dId.toUtf8().constData());
            });
            tblSessions_->setCellWidget(row, 7, cmbR);
            sessRows_.push_back({sid, sld, btnM, cmbR, fmtIt, 0, 0});
        }
    } else {
        for (uint32_t i = 0; i < ns; ++i) {
            const auto& s = st.sessions[i];
            auto& rw = sessRows_[i];
            const QString fmtS = QStringLiteral("%1k/%2ch").arg(s.sample_rate / 1000.0, 0, 'g', 4).arg(s.channels);
            if (rw.fmtItem->text() != fmtS) rw.fmtItem->setText(fmtS);
            if (!rw.volSlider->isSliderDown() && tNow >= rw.holdUntilMs) {
                rw.volSlider->blockSignals(true);
                rw.volSlider->setValue(static_cast<int>(s.volume_milli / 10));
                rw.volSlider->blockSignals(false);
            }
            rw.btnMute->blockSignals(true);
            rw.btnMute->setChecked(s.muted != 0);
            rw.btnMute->setText(s.muted ? QStringLiteral("Muted") : QStringLiteral("Mute"));
            rw.btnMute->blockSignals(false);
            if (!rw.cmbRoute->view()->isVisible()) {
                const QString tgt = cFixedStr(s.target_device, sizeof(s.target_device));
                if (int ri = rw.cmbRoute->findData(tgt); ri >= 0 && rw.cmbRoute->currentIndex() != ri) {
                    rw.cmbRoute->blockSignals(true);
                    rw.cmbRoute->setCurrentIndex(ri);
                    rw.cmbRoute->blockSignals(false);
                }
            }
        }
    }

    // Feed live Auto-Graph topology canvas when visible
    if (introWin_ && introWin_->isVisible() && graphCanvas_) {
        AutoGraphSnapshotData ag;
        for (uint32_t i = 0; i < ns; ++i) {
            const QString nm = cFixedStr(st.sessions[i].client_name, sizeof(st.sessions[i].client_name));
            if (st.sessions[i].is_capture) ag.captApps.append(nm);
            else ag.playApps.append(nm);
        }
        ag.hwSink = hwDev;
        ag.hwMic = hwCap;
        ag.midiCtrl = cFixedStr(st.midi_device, sizeof(st.midi_device));
        ag.rate = st.hw_sample_rate;
        ag.period = st.hw_period_frames;
        ag.bits = st.hw_format_bits;
        ag.spkVol = st.desktop_volume_pct;
        ag.spkMuted = st.desktop_muted != 0;
        ag.micVol = st.desktop_mic_volume_pct;
        ag.micMuted = st.desktop_mic_muted != 0;
        ag.btActive = st.bt_active != 0;
        ag.btCodec = cFixedStr(st.bt_codec, sizeof(st.bt_codec));
        ag.btDelayMs = st.bt_delay_ms;
        ag.midiCount = st.midi_events_routed;
        ag.eqEnabled = st.eq_cfg.enabled != 0;
        ag.eqMode = st.eq_cfg.dynamic_mode;
        ag.eqPreset = cFixedStr(st.eq_cfg.preset_name, sizeof(st.eq_cfg.preset_name));
        ag.limiterEnabled = st.eq_cfg.limiter_enabled != 0;
        ag.crossfeedEnabled = st.param_eq.crossfeed_enabled != 0;
        ag.capSampleRate = st.hw_sample_rate;
        ag.hwArch = cFixedStr(st.hw_analog_info.card_arch_name, sizeof(st.hw_analog_info.card_arch_name));
        ag.hwAmpTier = st.hw_analog_cfg.hp_amp_tier;
        ag.hwDacUnity = st.hw_analog_info.dac_unity_active != 0;
        ag.hwHidConnected = st.hw_analog_info.hid_box_connected != 0;
        ag.paClients = st.pa_clients;
        ag.paRunning = st.pa_server_running != 0;

        if (studioDbgHandle_) {
            const QJsonObject snap = parseJsonObj(SalsaStr(salsa_dbg_latest_snapshot_json(studioDbgHandle_)).toQString());
            if (!snap.isEmpty()) {
                ag.hasSnap = true;
                const QJsonObject rt = snap.value(QStringLiteral("rt")).toObject();
                const QJsonObject pw = snap.value(QStringLiteral("pw")).toObject();
                ag.rtAvgUs = rt.value(QStringLiteral("avg_cycle_us")).toDouble(0.0);
                ag.rtLastUs = rt.value(QStringLiteral("last_cycle_us")).toDouble(0.0);
                const auto pwBool = [&pw](const QString& k) {
                    const QJsonValue v = pw.value(k);
                    return v.toBool(false) || (v.toInteger(0) != 0);
                };
                ag.pwBrokerRunning = pwBool(QStringLiteral("broker_running"));
                ag.pwActiveCasting = pwBool(QStringLiteral("active_casting"));
                ag.pwFramesRelayed = static_cast<uint64_t>(pw.value(QStringLiteral("frames_relayed_total")).toInteger(0));
                ag.pwFramesDropped = static_cast<uint64_t>(pw.value(QStringLiteral("frames_dropped")).toInteger(0));
                ag.pwDmabufStreams = static_cast<uint32_t>(pw.value(QStringLiteral("dmabuf_streams")).toInteger(0));
                ag.pwMemfdStreams = static_cast<uint32_t>(pw.value(QStringLiteral("memfd_streams")).toInteger(0));
                ag.pwAudioRejects = static_cast<uint32_t>(pw.value(QStringLiteral("audio_rejects")).toInteger(0));
                ag.pwCastLimitW = static_cast<uint32_t>(pw.value(QStringLiteral("cast_limit_w")).toInteger(0));
                ag.pwCastLimitH = static_cast<uint32_t>(pw.value(QStringLiteral("cast_limit_h")).toInteger(0));
                ag.pwCastLimitFps = static_cast<uint32_t>(pw.value(QStringLiteral("cast_limit_fps")).toInteger(0));
                ag.pwActualFps = pw.value(QStringLiteral("actual_fps_x100")).toDouble(0.0) / 100.0;
                ag.pwSocketPath = pw.value(QStringLiteral("socket_path")).toString();
                ag.pwClients = pw.value(QStringLiteral("clients")).toArray();
            }
        }
        graphCanvas_->updateData(ag);
    }

    QWidget* visWin = (mainWin_ && mainWin_->isVisible() && !mainWin_->isMinimized())
                          ? static_cast<QWidget*>(mainWin_)
                          : ((advWin_ && advWin_->isVisible() && !advWin_->isMinimized())
                                 ? static_cast<QWidget*>(advWin_)
                                 : nullptr);
    const int wantMs = visWin ? salsaTelemetryIntervalMs(visWin, 120) : 450;
    if (pollTimer_ && pollTimer_->interval() != wantMs) {
        pollTimer_->setInterval(wantMs);
    }
}

void StudioController::dumpAuditPngs(const QString& outDir) {
    QDir().mkpath(outDir);
    pollEngine(true);
    if (mainWin_) {
        mainWin_->show();
        QApplication::processEvents();
        QApplication::processEvents();
    }
    if (advWin_) {
        advWin_->show();
        QApplication::processEvents();
        QApplication::processEvents();
    }
    const QSize mainMin = mainWin_ ? mainWin_->minimumSizeHint() : QSize();
    const QSize advMin = advWin_ ? advWin_->minimumSizeHint() : QSize();
    const QSize mainBefore = mainWin_ ? mainWin_->size() : QSize();
    const QSize advBefore = advWin_ ? advWin_->size() : QSize();
    flashSaveFeedback(
        QStringLiteral("Saved (16:10:00): %1").arg(SalsaStr(salsa_settings_path_str()).toQString()),
        true);
    QApplication::processEvents();
    QApplication::processEvents();
    const QSize mainAfter = mainWin_ ? mainWin_->size() : QSize();
    const QSize advAfter = advWin_ ? advWin_->size() : QSize();
    std::printf(
        "AUDIT_SIZE main=%dx%d->%dx%d (min=%dx%d) adv=%dx%d->%dx%d (min=%dx%d)\n",
        mainBefore.width(), mainBefore.height(), mainAfter.width(), mainAfter.height(),
        mainMin.width(), mainMin.height(),
        advBefore.width(), advBefore.height(), advAfter.width(), advAfter.height(),
        advMin.width(), advMin.height());
    if (mainWin_) mainWin_->grab().save(outDir + QStringLiteral("/main.png"));
    if (advWin_) advWin_->grab().save(outDir + QStringLiteral("/advanced.png"));
    if (introWin_) {
        introWin_->show();
        QApplication::processEvents();
        QApplication::processEvents();
        introWin_->grab().save(outDir + QStringLiteral("/topology.png"));
    }
}

}  // namespace salsa

#include "salsa_studio.moc"

int main(int argc, char** argv) {
    salsa_reset_sigpipe();
    if (const int cliRc = salsa::handleLegalCliArgs(argc, argv, "salsa-studio"); cliRc >= 0) {
        return cliRc;
    }
    bool selfTest = false;
    QString renderAuditDir;
    bool forceStudio = false;
    bool forceTopology = false;
    bool forceAdvanced = false;
    bool forceThemeStudio = false;
    bool forceAbout = false;
    for (int i = 1; i < argc; ++i) {
        const QString arg = QString::fromUtf8(argv[i]);
        if (arg == QLatin1String("--self-test")) selfTest = true;
        else if (arg == QLatin1String("--render-audit") && i + 1 < argc) renderAuditDir = QString::fromUtf8(argv[++i]);
        else if (arg == QLatin1String("--studio")) forceStudio = true;
        else if (arg == QLatin1String("--topology") || arg == QLatin1String("--graph")) forceTopology = true;
        else if (arg == QLatin1String("--advanced")) forceAdvanced = true;
        else if (arg == QLatin1String("--theme-studio")) forceThemeStudio = true;
        else if (arg == QLatin1String("--about")) forceAbout = true;
    }

    if (!salsa::verifyBridgeAbi()) {
        std::fprintf(stderr, "FATAL: Salsa ABI struct size mismatch between C++ and Rust\n");
        return 2;
    }

    if (selfTest) {
        qputenv("QT_QPA_PLATFORM", "offscreen");
        QApplication app(argc, argv);
        const salsa::SalsaStr st(salsa_studio_selftest());
        const QString res = st.toQString();
        if (!res.startsWith(QLatin1String("OK:"))) {
            std::fprintf(stderr, "%s\n", res.toUtf8().constData());
            return 1;
        }
        salsa::StudioController ctrl(true);
        std::printf("%s\n", res.toUtf8().constData());
        return 0;
    }

    if (!renderAuditDir.isEmpty()) {
        qputenv("QT_QPA_PLATFORM", "offscreen");
        QApplication app(argc, argv);
        salsa::StudioController ctrl(true);
        ctrl.dumpAuditPngs(renderAuditDir);
        return 0;
    }

    const QString mode = forceAdvanced
                             ? QStringLiteral("advanced")
                             : (forceThemeStudio
                                    ? QStringLiteral("theme-studio")
                                    : (forceAbout
                                           ? QStringLiteral("about")
                                           : (forceTopology
                                                  ? QStringLiteral("topology")
                                                  : (forceStudio ? QStringLiteral("studio") : QStringLiteral("auto")))));
    if (salsa::tryActivateExistingSingleInstance(
            QStringLiteral("salsa-studio.sock"),
            mode,
            qgetenv("XDG_ACTIVATION_TOKEN"))) {
        return 0;
    }

    QApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("salsa-studio"));
    app.setDesktopFileName(QStringLiteral("salsa-lwasapi"));

    salsa::StudioController ctrl(forceStudio);
    ctrl.start(forceStudio, forceTopology, forceAdvanced, forceThemeStudio, forceAbout);
    return app.exec();
}

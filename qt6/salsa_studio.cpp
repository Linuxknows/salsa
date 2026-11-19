#include "salsa_common.hpp"
#include "salsa_widgets.hpp"

#include <QAbstractItemView>
#include <QApplication>
#include <QCheckBox>
#include <QCloseEvent>
#include <QComboBox>
#include <QDialog>
#include <QDir>
#include <QDoubleSpinBox>
#include <QFileDialog>
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
#include <QPushButton>
#include <QScreen>
#include <QScrollArea>
#include <QShortcut>
#include <QSlider>
#include <QSpinBox>
#include <QSplitter>
#include <QTableWidget>
#include <QTextBrowser>
#include <QTimer>
#include <QVBoxLayout>

#include <algorithm>
#include <array>
#include <cmath>
#include <cstdio>

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

    void start(bool forceStudio, bool forceTopology = false);
    void shutdownAll();

private:
    void buildMainWindow();
    void buildTopologyWindow();
    void buildAdvancedWindow();

    void applyThemeAndFontAll(bool saveSettings = false);
    void syncFontScaleBars();
    void applyDynamicShortcuts();
    void populatePresetDropdown(const QString& selectTarget = QString());

    void pollEngine(bool initial);
    void checkSettingsFileSync();
    void syncGraphTimer();

    void pushEqToEngine();
    void saveStudioSettings();
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
    void onHwAnalogReset();

    void onVolStepChanged();
    void onSpkSliderMoved(int v);
    void onMicSliderMoved(int v);
    void onSessionSliderMoved(uint32_t sid, int v);
    void flushSessionVolume(uint32_t sid, int v);

    void openTopologyWindow();
    void enterStudioFromTopology();
    void openAdvancedWindow();
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
    QString lastDevSig_;
    QString currentManualKey_ = QStringLiteral("overview");

    qint64 eqHoldUntilMs_ = 0;
    qint64 btHoldUntilMs_ = 0;
    qint64 hwHoldUntilMs_ = 0;
    qint64 spkHoldUntilMs_ = 0;
    qint64 micHoldUntilMs_ = 0;

    // 3 Top-level windows + theme studio dialog
    QMainWindow* mainWin_ = nullptr;
    QWidget* introWin_ = nullptr;
    QWidget* advWin_ = nullptr;
    SalsaCustomThemeDialog* themeDlg_ = nullptr;

    // Main header widgets
    QLabel* lblTitle_ = nullptr;
    QLabel* lblVerPill_ = nullptr;
    QLabel* lblSubtitle_ = nullptr;
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
    QLabel* lblBtDiag_ = nullptr;
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
    QLabel* lblHwAnalogDiag_ = nullptr;
    QMap<QString, ShortcutCaptureEdit*> shortcutEdits_;
    std::vector<QShortcut*> activeShortcuts_;

    // Timers
    QTimer* pollTimer_ = nullptr;
    QTimer* graphAnimTimer_ = nullptr;
    QTimer* spkSendTimer_ = nullptr;
    QTimer* micSendTimer_ = nullptr;
    QTimer* dbgSnapTimer_ = nullptr;
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
    currentThemeName_ = guiSettings_.value(QStringLiteral("theme")).toString(QStringLiteral("Obsidian Glass"));
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

    // Restore persisted EQ & dynamic_mode immediately
    if (guiSettings_.contains(QStringLiteral("eq"))) {
        const QByteArray eqJs = jsonCompact(guiSettings_.value(QStringLiteral("eq")).toObject()).toUtf8();
        salsa_json_to_eq_cfg(eqJs.constData(), &eqCfg_, &paramExt_);
    }

    // Apply initial EQ controls from restored eqCfg_/paramExt_
    updatingUi_ = true;
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

    applyThemeAndFontAll(false);
    applyDynamicShortcuts();

    spkSendTimer_ = new QTimer(this);
    spkSendTimer_->setSingleShot(true);
    spkSendTimer_->setInterval(40);
    connect(spkSendTimer_, &QTimer::timeout, this, [this]() {
        spkHoldUntilMs_ = nowMs() + 350;
        salsa_set_master_volume(sldSpk_->value(), status_.desktop_muted ? 1 : 0);
    });

    micSendTimer_ = new QTimer(this);
    micSendTimer_->setSingleShot(true);
    micSendTimer_->setInterval(40);
    connect(micSendTimer_, &QTimer::timeout, this, [this]() {
        micHoldUntilMs_ = nowMs() + 350;
        salsa_set_mic_volume(sldMic_->value(), status_.desktop_mic_muted ? 1 : 0);
    });

    pollTimer_ = new QTimer(this);
    pollTimer_->setInterval(500);
    connect(pollTimer_, &QTimer::timeout, this, [this]() {
        pollEngine(false);
        checkSettingsFileSync();
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
    if (pollTimer_) pollTimer_->stop();
    if (graphAnimTimer_) graphAnimTimer_->stop();
    if (dbgSnapTimer_) dbgSnapTimer_->stop();
    if (studioDbgHandle_) {
        salsa_dbg_destroy(studioDbgHandle_);
        studioDbgHandle_ = nullptr;
    }
    if (themeDlg_) { themeDlg_->close(); delete themeDlg_; themeDlg_ = nullptr; }
    if (advWin_) { advWin_->close(); }
    if (introWin_) { introWin_->close(); }
    if (mainWin_) { mainWin_->close(); }
}

void StudioController::start(bool forceStudio, bool forceTopology) {
    forceStudio_ = forceStudio;
    pollEngine(true);
    // Honor show_topology_intro unless --studio or --topology explicitly passed
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
    lblVerPill_ = new QLabel(QStringLiteral("V1.19.0"), topCard);
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
    connect(cmbRate_, &QComboBox::currentIndexChanged, this, [this]() { onClockChanged(); });
    rateCol->addWidget(cmbRate_);
    lblClockSrc_ = new QLabel(QStringLiteral("Clock: 48kHz / 32-bit"), topCard);
    lblClockSrc_->setObjectName(QStringLiteral("BandDynLabel"));
    rateCol->addWidget(lblClockSrc_);
    rateCol->addStretch(1);
    topLay->addLayout(rateCol);

    auto* periodCol = new QVBoxLayout();
    periodCol->addWidget(new QLabel(QStringLiteral("Quantum & Mic Rate:"), topCard));
    cmbPeriod_ = new QComboBox(topCard);
    for (int p : {64, 128, 256, 512, 1024, 2048}) cmbPeriod_->addItem(QStringLiteral("%1 frames").arg(p), p);
    cmbPeriod_->setCurrentIndex(2);
    connect(cmbPeriod_, &QComboBox::currentIndexChanged, this, [this]() { onClockChanged(); });
    periodCol->addWidget(cmbPeriod_);
    cmbCapRate_ = new QComboBox(topCard);
    cmbCapRate_->addItem(QStringLiteral("Mic: Auto (Master)"), 0);
    for (int r : {44100, 48000, 88200, 96000, 176400, 192000}) {
        cmbCapRate_->addItem(QStringLiteral("Mic: %1 Hz").arg(QLocale().toString(r)), r);
    }
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
    connect(cmbBits_, &QComboBox::currentIndexChanged, this, [this]() { onClockChanged(); });
    bitsCol->addWidget(cmbBits_);
    bitsCol->addStretch(1);
    topLay->addLayout(bitsCol);

    auto* themeCol = new QVBoxLayout();
    themeCol->addWidget(new QLabel(QStringLiteral("Theme:"), topCard));
    cmbTheme_ = new QComboBox(topCard);
    for (const QString& tn : allThemeNames(false)) cmbTheme_->addItem(tn);
    cmbTheme_->setCurrentText(currentThemeName_);
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
    connect(fontBarMain_, &FontScaleBar::fontScaleChanged, this, [this](int p) {
        fontPct_ = p;
        salsa_save_font_scale(fontPct_);
        syncFontScaleBars();
        applyThemeAndFontAll(false);
    });
    engTopRow->addWidget(fontBarMain_);
    engCol->addLayout(engTopRow);

    auto* hdrBtns = new QHBoxLayout();
    hdrBtns->setSpacing(6);
    auto* btnTopo = new QPushButton(QStringLiteral("Topology (F1)"), topCard);
    connect(btnTopo, &QPushButton::clicked, this, [this]() { openTopologyWindow(); });
    hdrBtns->addWidget(btnTopo);
    auto* btnAdv = new QPushButton(QStringLiteral("Advanced Options (Ctrl+,)"), topCard);
    btnAdv->setObjectName(QStringLiteral("PrimaryBtn"));
    connect(btnAdv, &QPushButton::clicked, this, [this]() { openAdvancedWindow(); });
    hdrBtns->addWidget(btnAdv);
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
    connect(sldPreamp_, &QSlider::valueChanged, this, [this]() { onDynamicsSliderChanged(); });
    dynRow->addWidget(sldPreamp_);

    dynRow->addSpacing(12);
    lblThresh_ = new QLabel(QStringLiteral("Dyn Threshold: -18.0 dB"), eqCard);
    dynRow->addWidget(lblThresh_);
    sldThresh_ = new QSlider(Qt::Horizontal, eqCard);
    sldThresh_->setRange(-480, 0);
    sldThresh_->setValue(-180);
    connect(sldThresh_, &QSlider::valueChanged, this, [this]() { onDynamicsSliderChanged(); });
    dynRow->addWidget(sldThresh_);

    dynRow->addSpacing(12);
    lblRatio_ = new QLabel(QStringLiteral("Dyn Ratio: 2.5:1"), eqCard);
    dynRow->addWidget(lblRatio_);
    sldRatio_ = new QSlider(Qt::Horizontal, eqCard);
    sldRatio_->setRange(10, 80);
    sldRatio_->setValue(25);
    connect(sldRatio_, &QSlider::valueChanged, this, [this]() { onDynamicsSliderChanged(); });
    dynRow->addWidget(sldRatio_);

    dynRow->addSpacing(16);
    lblPeaks_ = new QLabel(QStringLiteral("Output Peak L/R: -60.0 / -60.0 dBFS"), eqCard);
    lblPeaks_->setObjectName(QStringLiteral("PeaksLabel"));
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
    connect(cmbVolStep_, &QComboBox::currentIndexChanged, this, [this]() { onVolStepChanged(); });
    hwHdrRow->addWidget(cmbVolStep_);
    hwCol->addLayout(hwHdrRow);

    lblEndpoints_ = new QLabel(QStringLiteral("Sink: Detecting... | Mic: Detecting... | PA v35: Initializing..."), hwCard);
    lblEndpoints_->setObjectName(QStringLiteral("EndpointsLabel"));
    lblEndpoints_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    hwCol->addWidget(lblEndpoints_);

    auto* devRow = new QHBoxLayout();
    devRow->addWidget(new QLabel(QStringLiteral("Output Sink:"), hwCard));
    cmbSink_ = new QComboBox(hwCard);
    cmbSink_->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
    cmbSink_->setMinimumContentsLength(14);
    cmbSink_->addItem(QStringLiteral("Auto-Priority (BT A2DP > USB > Analog > HDMI)"), QStringLiteral("auto"));
    connect(cmbSink_, &QComboBox::activated, this, [this](int i) { onSinkActivated(i); });
    devRow->addWidget(cmbSink_, 1);
    btnBtReset_ = new QPushButton(QStringLiteral("Reset BT"), hwCard);
    connect(btnBtReset_, &QPushButton::clicked, this, [this]() { onBtResetClicked(); });
    devRow->addWidget(btnBtReset_);
    devRow->addWidget(new QLabel(QStringLiteral("Mic Input:"), hwCard));
    cmbSource_ = new QComboBox(hwCard);
    cmbSource_->setSizeAdjustPolicy(QComboBox::AdjustToMinimumContentsLengthWithIcon);
    cmbSource_->setMinimumContentsLength(14);
    cmbSource_->addItem(QStringLiteral("Auto-Priority (USB Mic > HDA > Virtual)"), QStringLiteral("auto"));
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
    connect(cmbBtCodec_, &QComboBox::currentIndexChanged, this, [this]() { if (!updatingUi_) onBtCfgChanged(); });
    btRow->addWidget(cmbBtCodec_);

    cmbBtLdac_ = new QComboBox(hwCard);
    cmbBtLdac_->addItem(QStringLiteral("LDAC ABR"), SALSA_LDAC_MODE_ABR);
    cmbBtLdac_->addItem(QStringLiteral("990 kbps"), SALSA_LDAC_MODE_990);
    cmbBtLdac_->addItem(QStringLiteral("660 kbps"), SALSA_LDAC_MODE_660);
    cmbBtLdac_->addItem(QStringLiteral("330 kbps"), SALSA_LDAC_MODE_330);
    connect(cmbBtLdac_, &QComboBox::currentIndexChanged, this, [this]() { if (!updatingUi_) onBtCfgChanged(); });
    btRow->addWidget(cmbBtLdac_);

    chkBtAbr_ = new QCheckBox(QStringLiteral("Wireless ABR"), hwCard);
    chkBtAbr_->setChecked(true);
    connect(chkBtAbr_, &QCheckBox::toggled, this, [this]() { if (!updatingUi_) onBtCfgChanged(); });
    btRow->addWidget(chkBtAbr_);

    chkBtSco_ = new QCheckBox(QStringLiteral("HFP Mic"), hwCard);
    connect(chkBtSco_, &QCheckBox::toggled, this, [this]() { if (!updatingUi_) onBtCfgChanged(); });
    btRow->addWidget(chkBtSco_);

    lblBtDiag_ = new QLabel(QStringLiteral("BT: Standby"), hwCard);
    lblBtDiag_->setObjectName(QStringLiteral("BandDynLabel"));
    lblBtDiag_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    btRow->addWidget(lblBtDiag_, 1);
    hwCol->addLayout(btRow);

    auto* spkRow = new QHBoxLayout();
    lblSpk_ = new QLabel(QStringLiteral("Speaker Vol (80%):"), hwCard);
    lblSpk_->setMinimumWidth(135);
    spkRow->addWidget(lblSpk_);
    sldSpk_ = new StepVolumeSlider([this]() { return volStepPct_; }, Qt::Horizontal, hwCard);
    sldSpk_->setRange(0, 150);
    sldSpk_->setValue(80);
    connect(sldSpk_, &QSlider::valueChanged, this, [this](int v) { if (!updatingUi_) onSpkSliderMoved(v); });
    connect(sldSpk_, &QSlider::sliderReleased, this, [this]() { spkSendTimer_->start(0); });
    spkRow->addWidget(sldSpk_, 1);
    btnSpkMute_ = new QPushButton(QStringLiteral("Mute Spk"), hwCard);
    connect(btnSpkMute_, &QPushButton::clicked, this, [this]() {
        const int nextMute = status_.desktop_muted ? 0 : 1;
        spkHoldUntilMs_ = nowMs() + 350;
        salsa_set_master_volume(sldSpk_->value(), nextMute);
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
    connect(sldMic_, &QSlider::valueChanged, this, [this](int v) { if (!updatingUi_) onMicSliderMoved(v); });
    connect(sldMic_, &QSlider::sliderReleased, this, [this]() { micSendTimer_->start(0); });
    micRow->addWidget(sldMic_, 1);
    btnMicMute_ = new QPushButton(QStringLiteral("Mute Mic"), hwCard);
    connect(btnMicMute_, &QPushButton::clicked, this, [this]() {
        const int nextMute = status_.desktop_mic_muted ? 0 : 1;
        micHoldUntilMs_ = nowMs() + 350;
        salsa_set_mic_volume(sldMic_->value(), nextMute);
        status_.desktop_mic_muted = nextMute;
        btnMicMute_->setText(nextMute ? QStringLiteral("Unmute Mic") : QStringLiteral("Mute Mic"));
    });
    micRow->addWidget(btnMicMute_);
    hwCol->addLayout(micRow);

    lblMidiStatus_ = new QLabel(QStringLiteral("DAW MIDI Hub: Auto-Detecting (0 key presses)"), hwCard);
    lblMidiStatus_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
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
        QStringLiteral("✔ V1.19.0 LIVE | MASTER CLOCK | HW ANALOG DAC | DYNAMIC EQ | BT ABR"),
        graphCard);
    lblAutoBadge->setObjectName(QStringLiteral("AutoBadgeLabel"));
    graphHdrRow->addWidget(lblAutoBadge);

    fontBarIntro_ = new FontScaleBar(fontPct_, true, graphCard);
    connect(fontBarIntro_, &FontScaleBar::fontScaleChanged, this, [this](int p) {
        fontPct_ = p;
        salsa_save_font_scale(fontPct_);
        syncFontScaleBars();
        applyThemeAndFontAll(false);
    });
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
        connect(b, &QPushButton::clicked, this, [this, key]() { selectManualChapter(key); });
        manualBtns_.insert(key, b);
        chapBar->addWidget(b);
    }

    auto* btnSync = new QPushButton(QStringLiteral("Sync Now"), manCard);
    btnSync->setObjectName(QStringLiteral("PrimaryBtn"));
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

    auto* btnAdv = new QPushButton(QStringLiteral("Advanced Options (Ctrl+,)"), introFooter);
    connect(btnAdv, &QPushButton::clicked, this, [this]() { openAdvancedWindow(); });
    footRow->addWidget(btnAdv);

    auto* btnEnter = new QPushButton(QStringLiteral("Enter Studio Mixer  ->"), introFooter);
    btnEnter->setObjectName(QStringLiteral("PrimaryBtn"));
    btnEnter->setMinimumHeight(32);
    connect(btnEnter, &QPushButton::clicked, this, [this]() { enterStudioFromTopology(); });
    footRow->addWidget(btnEnter);
    graphLayout->addWidget(introFooter);

    selectManualChapter(QStringLiteral("overview"));
}

void StudioController::buildAdvancedWindow() {
    advWin_ = new PopToolWindow(
        QStringLiteral("Salsa — Advanced Options, DSP, Hardware Analog & Diagnostics"),
        [this](QCloseEvent*) {
            if (closingAll_) return true;
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
    connect(fontBarAdv_, &FontScaleBar::fontScaleChanged, this, [this](int p) {
        fontPct_ = p;
        salsa_save_font_scale(fontPct_);
        syncFontScaleBars();
        applyThemeAndFontAll(false);
    });
    hcLay->addWidget(fontBarAdv_);

    auto* btnCloseAdv = new QPushButton(QStringLiteral("Close (Esc)"), hdrCard);
    connect(btnCloseAdv, &QPushButton::clicked, advWin_, &QWidget::hide);
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
    connect(btnOpenDbg, &QPushButton::clicked, this, [this]() { openLiveDebuggerWindow(); });
    gDiag->addWidget(btnOpenDbg, 0, 0, 1, 2);

    auto* btnBench = new QPushButton(QStringLiteral("Run Latency Benchmark"), diagCard);
    connect(btnBench, &QPushButton::clicked, this, [this]() {
        const SalsaStr rep(salsa_run_latency_bench());
        showReportDialog(advWin_, QStringLiteral("Salsa Latency Benchmark"), rep.toQString(), currentTheme_, fontPct_);
    });
    gDiag->addWidget(btnBench, 1, 0);

    auto* btnAudit = new QPushButton(QStringLiteral("Run System Health Audit"), diagCard);
    connect(btnAudit, &QPushButton::clicked, this, [this]() {
        const QJsonObject rep = parseJsonObj(SalsaStr(salsa_run_health_audit_json()).toQString());
        const QString pretty = QString::fromUtf8(QJsonDocument(rep).toJson(QJsonDocument::Indented));
        showReportDialog(advWin_, QStringLiteral("Salsa System Health & Process Audit"), pretty, currentTheme_, fontPct_);
    });
    gDiag->addWidget(btnAudit, 1, 1);

    auto* btnSync = new QPushButton(QStringLiteral("Sync Audio Modules Now"), diagCard);
    connect(btnSync, &QPushButton::clicked, this, [this]() {
        const SalsaStr rep(salsa_run_sync_modules(true));
        showReportDialog(advWin_, QStringLiteral("Salsa Module Auto-Sync Report"), rep.toQString(), currentTheme_, fontPct_, 760, 360);
    });
    gDiag->addWidget(btnSync, 2, 0);

    auto* btnMidi = new QPushButton(QStringLiteral("Send Test MIDI (C4)"), diagCard);
    connect(btnMidi, &QPushButton::clicked, this, []() { salsa_send_test_midi(60, 115); });
    gDiag->addWidget(btnMidi, 2, 1);

    btnRstEngine_ = new QPushButton(QStringLiteral("Restart Audio Engine"), diagCard);
    connect(btnRstEngine_, &QPushButton::clicked, this, [this]() { runSystemdRestart(QStringLiteral("lwasapi-audiodg.service"), btnRstEngine_); });
    gDiag->addWidget(btnRstEngine_, 3, 0);

    btnRstTray_ = new QPushButton(QStringLiteral("Restart Tray Service"), diagCard);
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
    connect(btnTogglePacman_, &QPushButton::clicked, this, [this]() {
        const SalsaStr msg(salsa_toggle_pacman_hook());
        refreshAdvIntegration();
        if (!msg.empty()) QMessageBox::information(advWin_, QStringLiteral("Pacman Hook"), msg.toQString());
    });
    hookRow->addWidget(btnTogglePacman_, 1);

    chkAutosyncNow_ = new QCheckBox(QStringLiteral("Auto-Sync Watcher + Sync Now"), diagCard);
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
    connect(btnThemeEd, &QPushButton::clicked, this, [this]() { openCustomThemeDialog(); });
    rowApp->addWidget(btnThemeEd, 1);

    auto* btnResetSafe = new QPushButton(QStringLiteral("Reset Settings to Safe Defaults"), engCard);
    btnResetSafe->setObjectName(QStringLiteral("CompactBtn"));
    connect(btnResetSafe, &QPushButton::clicked, this, [this]() {
        onResetFlatClicked();
        onHwAnalogReset();
        chkShowIntroAdv_->setChecked(true);
    });
    rowApp->addWidget(btnResetSafe);
    ecLay->addLayout(rowApp);
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
    connect(chkLimiter_, &QCheckBox::toggled, this, [this]() { if (!updatingUi_) pushEqToEngine(); });
    rQa1->addWidget(chkLimiter_);

    chkEqPerDev_ = new QCheckBox(QStringLiteral("Per-Device EQ"), eqAdvCard);
    chkEqPerDev_->setChecked(guiSettings_.value(QStringLiteral("eq_per_device")).toBool(false));
    connect(chkEqPerDev_, &QCheckBox::toggled, this, [this](bool on) { onPerDeviceEqToggled(on); });
    rQa1->addWidget(chkEqPerDev_);
    rQa1->addStretch(1);
    qaLay->addLayout(rQa1);

    auto* rQa2 = new QHBoxLayout();
    auto* btnImp = new QPushButton(QStringLiteral("Import AutoEQ .txt"), eqAdvCard);
    btnImp->setObjectName(QStringLiteral("CompactBtn"));
    connect(btnImp, &QPushButton::clicked, this, [this]() { onImportAutoEqClicked(); });
    rQa2->addWidget(btnImp);

    auto* btnRel = new QPushButton(QStringLiteral("Reload AutoEQ .txt"), eqAdvCard);
    btnRel->setObjectName(QStringLiteral("CompactBtn"));
    connect(btnRel, &QPushButton::clicked, this, [this]() { onReloadAutoEqClicked(); });
    rQa2->addWidget(btnRel);
    rQa2->addStretch(1);
    qaLay->addLayout(rQa2);

    auto* rXf = new QHBoxLayout();
    chkCrossfeed_ = new QCheckBox(QStringLiteral("Bauer Crossfeed"), eqAdvCard);
    connect(chkCrossfeed_, &QCheckBox::toggled, this, [this]() {
        if (!updatingUi_) pushEqToEngine();
    });
    rXf->addWidget(chkCrossfeed_);

    sldCrossfeed_ = new QSlider(Qt::Horizontal, eqAdvCard);
    sldCrossfeed_->setRange(0, 100);
    sldCrossfeed_->setValue(30);
    connect(sldCrossfeed_, &QSlider::valueChanged, this, [this](int v) {
        lblCrossfeed_->setText(QStringLiteral("%1%").arg(v));
        if (!updatingUi_) pushEqToEngine();
    });
    rXf->addWidget(sldCrossfeed_, 1);
    lblCrossfeed_ = new QLabel(QStringLiteral("30%"), eqAdvCard);
    rXf->addWidget(lblCrossfeed_);

    cmbCrossfeedCut_ = new QComboBox(eqAdvCard);
    cmbCrossfeedCut_->addItem(QStringLiteral("650 Hz"), 650);
    cmbCrossfeedCut_->addItem(QStringLiteral("700 Hz"), 700);
    cmbCrossfeedCut_->addItem(QStringLiteral("850 Hz"), 850);
    cmbCrossfeedCut_->setCurrentIndex(1);
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
    connect(chkHwDacUnity_, &QCheckBox::toggled, this, [this]() { if (!updatingUi_) onHwAnalogCfgChanged(); });
    haR1->addWidget(chkHwDacUnity_);

    cmbHwAmp_ = new QComboBox(hwaCard);
    cmbHwAmp_->addItem(QStringLiteral("Amp: Auto (Preserve)"), SALSA_HW_AMP_AUTO);
    cmbHwAmp_->addItem(QStringLiteral("Amp: Low (<=32Ω IEM)"), SALSA_HW_AMP_LOW);
    cmbHwAmp_->addItem(QStringLiteral("Amp: Med (64-150Ω)"), SALSA_HW_AMP_MED);
    cmbHwAmp_->addItem(QStringLiteral("Amp: High (600Ω TPA6120)"), SALSA_HW_AMP_HIGH);
    connect(cmbHwAmp_, &QComboBox::currentIndexChanged, this, [this]() { if (!updatingUi_) onHwAnalogCfgChanged(); });
    haR1->addWidget(cmbHwAmp_);

    cmbHwRoute_ = new QComboBox(hwaCard);
    cmbHwRoute_->addItem(QStringLiteral("Route: Auto"), SALSA_HW_ROUTE_AUTO);
    cmbHwRoute_->addItem(QStringLiteral("Route: Headphones"), SALSA_HW_ROUTE_HP);
    cmbHwRoute_->addItem(QStringLiteral("Route: Line / Spk"), SALSA_HW_ROUTE_SPK);
    connect(cmbHwRoute_, &QComboBox::currentIndexChanged, this, [this]() { if (!updatingUi_) onHwAnalogCfgChanged(); });
    haR1->addWidget(cmbHwRoute_);

    chkHwRails_ = new QCheckBox(QStringLiteral("0dB Rails"), hwaCard);
    connect(chkHwRails_, &QCheckBox::toggled, this, [this]() { if (!updatingUi_) onHwAnalogCfgChanged(); });
    haR1->addWidget(chkHwRails_);
    haLay->addLayout(haR1);

    auto* haR2 = new QHBoxLayout();
    haR2->addWidget(new QLabel(QStringLiteral("Analog Drive:"), hwaCard));
    sldHwBoost_ = new QSlider(Qt::Horizontal, hwaCard);
    sldHwBoost_->setRange(0, 24);
    connect(sldHwBoost_, &QSlider::valueChanged, this, [this](int v) {
        lblHwBoost_->setText(QStringLiteral("+%1 dB").arg(v * 0.5, 0, 'f', 1));
        if (!updatingUi_ && !sldHwBoost_->isSliderDown()) onHwAnalogCfgChanged();
    });
    connect(sldHwBoost_, &QSlider::sliderReleased, this, [this]() { onHwAnalogCfgChanged(); });
    haR2->addWidget(sldHwBoost_, 1);
    lblHwBoost_ = new QLabel(QStringLiteral("+0.0 dB"), hwaCard);
    lblHwBoost_->setMinimumWidth(56);
    haR2->addWidget(lblHwBoost_);
    haLay->addLayout(haR2);

    auto* haR3 = new QHBoxLayout();
    haR3->addWidget(new QLabel(QStringLiteral("Mic HW Boost:"), hwaCard));
    sldHwMicBoost_ = new QSlider(Qt::Horizontal, hwaCard);
    sldHwMicBoost_->setRange(0, 24);
    connect(sldHwMicBoost_, &QSlider::valueChanged, this, [this](int v) {
        lblHwMicBoost_->setText(QStringLiteral("+%1 dB").arg(static_cast<double>(v), 0, 'f', 1));
        if (!updatingUi_ && !sldHwMicBoost_->isSliderDown()) onHwAnalogCfgChanged();
    });
    connect(sldHwMicBoost_, &QSlider::sliderReleased, this, [this]() { onHwAnalogCfgChanged(); });
    haR3->addWidget(sldHwMicBoost_, 1);
    lblHwMicBoost_ = new QLabel(QStringLiteral("+0.0 dB"), hwaCard);
    lblHwMicBoost_->setMinimumWidth(56);
    haR3->addWidget(lblHwMicBoost_);

    auto* btnSafeHw = new QPushButton(QStringLiteral("Safe Defaults"), hwaCard);
    connect(btnSafeHw, &QPushButton::clicked, this, [this]() { onHwAnalogReset(); });
    haR3->addWidget(btnSafeHw);
    haLay->addLayout(haR3);

    lblHwAnalogDiag_ = new QLabel(QStringLiteral("HW Stage: Probing..."), hwaCard);
    lblHwAnalogDiag_->setObjectName(QStringLiteral("BandDynLabel"));
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
    if (tblSessions_) {
        tblSessions_->setColumnWidth(5, scaledPx(96, fontPct_));
        tblSessions_->setColumnWidth(7, scaledPx(126, fontPct_));
    }
    selectManualChapter(currentManualKey_);
    if (advWin_ && advWin_->isVisible()) refreshAdvIntegration();

    if (saveSettings) {
        updateGuiSettingsPatch(QJsonObject{
            {QStringLiteral("theme"), currentThemeName_},
            {QStringLiteral("font_scale_pct"), fontPct_},
        });
    }
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
    addSc(QStringLiteral("refresh"), [this]() { pollEngine(false); });
    addSc(QStringLiteral("quit"), [this]() { shutdownAll(); });
    addSc(QStringLiteral("reset_flat"), [this]() { onResetFlatClicked(); });

    // Universal Zoom shortcuts across Studio windows
    for (QWidget* host : std::array<QWidget*, 3>{mainWin_, introWin_, advWin_}) {
        if (!host) continue;
        auto* scPlus = new QShortcut(QKeySequence::ZoomIn, host);
        connect(scPlus, &QShortcut::activated, this, [this]() {
            fontPct_ = normalizeFontScalePct(fontPct_ + 10);
            salsa_save_font_scale(fontPct_);
            syncFontScaleBars();
            applyThemeAndFontAll(false);
        });
        activeShortcuts_.push_back(scPlus);

        auto* scMinus = new QShortcut(QKeySequence::ZoomOut, host);
        connect(scMinus, &QShortcut::activated, this, [this]() {
            fontPct_ = normalizeFontScalePct(fontPct_ - 10);
            salsa_save_font_scale(fontPct_);
            syncFontScaleBars();
            applyThemeAndFontAll(false);
        });
        activeShortcuts_.push_back(scMinus);

        auto* scReset = new QShortcut(QKeySequence(QStringLiteral("Ctrl+0")), host);
        connect(scReset, &QShortcut::activated, this, [this]() {
            fontPct_ = 100;
            salsa_save_font_scale(100);
            syncFontScaleBars();
            applyThemeAndFontAll(false);
        });
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
        themeDlg_ = new SalsaCustomThemeDialog(currentTheme_, fontPct_, mainWin_);
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
            currentThemeName_ = QString::fromLatin1(CUSTOM_THEME_NAME);
            if (cmbTheme_) cmbTheme_->setCurrentText(currentThemeName_);
            updateGuiSettingsPatch(QJsonObject{
                {QStringLiteral("theme"), currentThemeName_},
                {QStringLiteral("custom_theme"), themeToJsonObj(currentTheme_)},
            });
            applyThemeAndFontAll(true);
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
    saveStudioSettings();
}

void StudioController::saveStudioSettings() {
    const QJsonObject eqObj = parseJsonObj(SalsaStr(salsa_eq_cfg_to_json(&eqCfg_, &paramExt_)).toQString());
    QJsonObject payload;
    payload.insert(QStringLiteral("rate"), qvarInt(cmbRate_->currentData(), 48000));
    payload.insert(QStringLiteral("period"), qvarInt(cmbPeriod_->currentData(), 256));
    payload.insert(QStringLiteral("bits"), qvarInt(cmbBits_->currentData(), 32));
    payload.insert(QStringLiteral("capture_sample_rate"), qvarInt(cmbCapRate_->currentData(), 0));
    payload.insert(QStringLiteral("eq"), eqObj);
    payload.insert(QStringLiteral("imported_eq_presets"), savedAutoEq_);
    payload.insert(QStringLiteral("eq_per_device"), chkEqPerDev_->isChecked());
    payload.insert(QStringLiteral("theme"), currentThemeName_);
    payload.insert(QStringLiteral("font_scale_pct"), fontPct_);
    payload.insert(QStringLiteral("volume_step_pct"), volStepPct_);
    if (chkEqPerDev_->isChecked() && !lastSinkId_.isEmpty()) {
        savePerDeviceEqForSink(lastSinkId_);
    }
    salsa_save_studio_settings_json(jsonCompact(payload).toUtf8().constData());
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
    if (chkEqPerDev_->isChecked() && !lastSinkId_.isEmpty()) savePerDeviceEqForSink(lastSinkId_);
    salsa_set_devices(targetId.toUtf8().constData(), nullptr);
    lastSinkId_ = targetId;
    if (chkEqPerDev_->isChecked()) loadPerDeviceEqForSink(targetId);
}

void StudioController::onSourceActivated(int idx) {
    const QString targetId = cmbSource_->itemData(idx).toString();
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

void StudioController::onHwAnalogCfgChanged() {
    hwHoldUntilMs_ = nowMs() + 650;
    hwAnalog_.hw_dac_unity_vol = chkHwDacUnity_->isChecked() ? 1 : 0;
    hwAnalog_.hp_amp_tier = static_cast<uint32_t>(cmbHwAmp_->currentData().toInt());
    hwAnalog_.output_route = static_cast<uint32_t>(cmbHwRoute_->currentData().toInt());
    hwAnalog_.unlock_0db_rails = chkHwRails_->isChecked() ? 1 : 0;
    hwAnalog_.analog_boost_db = sldHwBoost_->value() * 0.5f;
    hwAnalog_.mic_hw_boost_db = static_cast<float>(sldHwMicBoost_->value());
    salsa_set_hw_analog(&hwAnalog_);
}

void StudioController::onHwAnalogReset() {
    updatingUi_ = true;
    chkHwDacUnity_->setChecked(true);
    cmbHwAmp_->setCurrentIndex(0);
    cmbHwRoute_->setCurrentIndex(0);
    chkHwRails_->setChecked(false);
    sldHwBoost_->setValue(0);
    lblHwBoost_->setText(QStringLiteral("+0.0 dB"));
    sldHwMicBoost_->setValue(0);
    lblHwMicBoost_->setText(QStringLiteral("+0.0 dB"));
    updatingUi_ = false;
    onHwAnalogCfgChanged();
}

void StudioController::onVolStepChanged() {
    volStepPct_ = normalizeVolStepPct(qvarInt(cmbVolStep_->currentData(), 5));
    updateGuiSettingsPatch(QJsonObject{{QStringLiteral("volume_step_pct"), volStepPct_}});
}

void StudioController::onSpkSliderMoved(int v) {
    lblSpk_->setText(QStringLiteral("Speaker Vol (%1%):").arg(v));
    spkHoldUntilMs_ = nowMs() + 350;
    if (!spkSendTimer_->isActive()) spkSendTimer_->start(40);
}

void StudioController::onMicSliderMoved(int v) {
    lblMic_->setText(QStringLiteral("Mic Capture (%1%):").arg(v));
    micHoldUntilMs_ = nowMs() + 350;
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
    introWin_->show();
    introWin_->raise();
    introWin_->activateWindow();
    syncGraphTimer();
    pollEngine(false);
}

void StudioController::enterStudioFromTopology() {
    studioEntered_ = true;
    mainWin_->show();
    mainWin_->raise();
    mainWin_->activateWindow();
    if (introWin_) introWin_->hide();
    syncGraphTimer();
    pollEngine(false);
}

void StudioController::openAdvancedWindow() {
    refreshAdvIntegration();
    advWin_->show();
    advWin_->raise();
    advWin_->activateWindow();
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
    spawnDetachedWaylandBinary(QStringLiteral("salsa-debug"));
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
        const int diskPct = normalizeFontScalePct(guiSettings_.value(QStringLiteral("font_scale_pct")).toInt(fontPct_));
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
    }
}

void StudioController::pollEngine(bool initial) {
    SalsaEngineStatus st{};
    const bool gotSt = (salsa_get_engine_status(&st) == 0);
    SalsaDeviceList dl{};
    const bool gotDl = (salsa_list_devices(&dl) == 0);

    if (gotDl) {
        devices_ = dl;
        QString sig;
        for (uint32_t i = 0; i < std::min<uint32_t>(dl.num_devices, SALSA_MAX_DEVICES); ++i) {
            sig += cFixedStr(dl.devices[i].id, sizeof(dl.devices[i].id)) + QChar('|');
        }
        if (sig != lastDevSig_) {
            lastDevSig_ = sig;
            updatingUi_ = true;
            cmbSink_->clear();
            cmbSink_->addItem(QStringLiteral("Auto-Priority (BT A2DP > USB > Analog > HDMI)"), QStringLiteral("auto"));
            cmbSource_->clear();
            cmbSource_->addItem(QStringLiteral("Auto-Priority (USB Mic > HDA > Virtual)"), QStringLiteral("auto"));
            for (uint32_t i = 0; i < std::min<uint32_t>(dl.num_devices, SALSA_MAX_DEVICES); ++i) {
                const auto& d = dl.devices[i];
                const QString did = cFixedStr(d.id, sizeof(d.id));
                const QString dname = cFixedStr(d.name, sizeof(d.name));
                const QString tag = d.is_bluetooth ? QStringLiteral(" [Bluetooth]") : QStringLiteral(" [%1]").arg(cFixedStr(d.driver, sizeof(d.driver)));
                if (d.is_capture) cmbSource_->addItem(dname + tag, did);
                else cmbSink_->addItem(dname + tag, did);
            }
            updatingUi_ = false;
        }
    }

    if (!gotSt) {
        lblEndpoints_->setText(QStringLiteral("Audio engine offline — start lwasapi-audiodg.service to manage live streams"));
        return;
    }

    hasStatus_ = true;
    status_ = st;
    const qint64 tNow = nowMs();

    const QString hwDev = cFixedStr(st.hw_device, sizeof(st.hw_device));
    const QString hwCap = cFixedStr(st.hw_capture_device, sizeof(st.hw_capture_device));
    const QString actPbId = cFixedStr(st.active_playback_id, sizeof(st.active_playback_id));
    const QString actCapId = cFixedStr(st.active_capture_id, sizeof(st.active_capture_id));

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

    if (tNow >= spkHoldUntilMs_) {
        sldSpk_->setValue(static_cast<int>(st.desktop_volume_pct));
        lblSpk_->setText(QStringLiteral("Speaker Vol (%1%):").arg(st.desktop_volume_pct));
        btnSpkMute_->setText(st.desktop_muted ? QStringLiteral("Unmute Spk") : QStringLiteral("Mute Spk"));
    }
    if (tNow >= micHoldUntilMs_) {
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
            lblCrossfeed_->setText(QStringLiteral("%1%").arg(sldCrossfeed_->value()));
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
    const double pkL = st.output_peak_l > 1e-6f ? 20.0 * std::log10(st.output_peak_l) : -60.0;
    const double pkR = st.output_peak_r > 1e-6f ? 20.0 * std::log10(st.output_peak_r) : -60.0;
    lblPeaks_->setText(QStringLiteral("Output Peak L/R: %1 / %2 dBFS").arg(pkL, 0, 'f', 1).arg(pkR, 0, 'f', 1));

    if (tNow >= btHoldUntilMs_) {
        btCfg_ = st.bt_codec_cfg;
        if (int ci = cmbBtCodec_->findData(static_cast<int>(btCfg_.preferred_codec)); ci >= 0) cmbBtCodec_->setCurrentIndex(ci);
        if (int li = cmbBtLdac_->findData(static_cast<int>(btCfg_.ldac_mode)); li >= 0) cmbBtLdac_->setCurrentIndex(li);
        chkBtAbr_->setChecked(btCfg_.wireless_abr_enabled != 0);
        chkBtSco_->setChecked(btCfg_.hfp_sco_enabled != 0);
    }
    lblBtDiag_->setText(
        st.bt_active ? QStringLiteral("BT: %1 %2 kbps (%3 ms) %4")
                           .arg(cFixedStr(st.bt_codec, sizeof(st.bt_codec)))
                           .arg(st.bt_codec_cfg.effective_kbps)
                           .arg(st.bt_delay_ms)
                           .arg(cFixedStr(st.bt_codec_cfg.neg_reason, sizeof(st.bt_codec_cfg.neg_reason)))
                     : QStringLiteral("BT: Standby"));

    if (tNow >= hwHoldUntilMs_) {
        hwAnalog_ = st.hw_analog_cfg;
        chkHwDacUnity_->setChecked(hwAnalog_.hw_dac_unity_vol != 0);
        if (int ai = cmbHwAmp_->findData(static_cast<int>(hwAnalog_.hp_amp_tier)); ai >= 0) cmbHwAmp_->setCurrentIndex(ai);
        if (int ri = cmbHwRoute_->findData(static_cast<int>(hwAnalog_.output_route)); ri >= 0) cmbHwRoute_->setCurrentIndex(ri);
        chkHwRails_->setChecked(hwAnalog_.unlock_0db_rails != 0);
        sldHwBoost_->setValue(static_cast<int>(std::lround(hwAnalog_.analog_boost_db * 2.0f)));
        lblHwBoost_->setText(QStringLiteral("+%1 dB").arg(hwAnalog_.analog_boost_db, 0, 'f', 1));
        sldHwMicBoost_->setValue(static_cast<int>(std::lround(hwAnalog_.mic_hw_boost_db)));
        lblHwMicBoost_->setText(QStringLiteral("+%1 dB").arg(hwAnalog_.mic_hw_boost_db, 0, 'f', 1));
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
            rw.fmtItem->setText(QStringLiteral("%1k/%2ch").arg(s.sample_rate / 1000.0, 0, 'g', 4).arg(s.channels));
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
}

}  // namespace salsa

#include "salsa_studio.moc"

int main(int argc, char** argv) {
    salsa_reset_sigpipe();
    bool selfTest = false;
    bool forceStudio = false;
    bool forceTopology = false;
    for (int i = 1; i < argc; ++i) {
        const QString arg = QString::fromUtf8(argv[i]);
        if (arg == QLatin1String("--self-test")) selfTest = true;
        else if (arg == QLatin1String("--studio")) forceStudio = true;
        else if (arg == QLatin1String("--topology") || arg == QLatin1String("--graph")) forceTopology = true;
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

    QApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("salsa-studio"));
    app.setDesktopFileName(QStringLiteral("salsa-lwasapi"));

    salsa::StudioController ctrl(forceStudio);
    ctrl.start(forceStudio, forceTopology);
    return app.exec();
}

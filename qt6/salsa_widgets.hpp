#pragma once

#include "salsa_common.hpp"

#include <QCheckBox>
#include <QComboBox>
#include <QDialog>
#include <QFrame>
#include <QHash>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QPlainTextEdit>
#include <QProgressBar>
#include <QPushButton>
#include <QRectF>
#include <QSlider>
#include <QTabWidget>
#include <QTableWidget>
#include <QTimer>
#include <QWidget>

#include <array>
#include <vector>

namespace salsa {

struct AutoGraphSnapshotData {
    QStringList playApps;
    QStringList captApps;
    QString hwSink = QStringLiteral("Detecting...");
    QString hwMic = QStringLiteral("Detecting...");
    QString midiCtrl = QStringLiteral("Auto");
    uint32_t rate = 48000;
    uint32_t period = 256;
    uint32_t bits = 32;
    uint32_t spkVol = 80;
    bool spkMuted = false;
    uint32_t micVol = 100;
    bool micMuted = false;
    bool btActive = false;
    QString btCodec;
    uint32_t btDelayMs = 0;
    QString btNegReason;
    uint64_t midiCount = 0;
    bool eqEnabled = true;
    uint32_t eqMode = 1;
    QString eqPreset = QStringLiteral("Flat / Studio Reference");
    bool limiterEnabled = true;
    bool crossfeedEnabled = false;
    bool micApoEnabled = false;
    bool micRnnoiseActive = false;
    bool micSidetoneEnabled = false;
    uint32_t micSidetonePct = 25;
    uint32_t capSampleRate = 48000;
    QString hwArch;
    uint32_t hwAmpTier = 0;
    bool hwDacUnity = true;
    bool hwHidConnected = false;
    uint32_t paClients = 0;
    bool paRunning = false;
    // Live debug snapshot fields for SHM/RT/Video/PW
    bool hasSnap = false;
    double rtAvgUs = 0.0;
    double rtLastUs = 0.0;
    bool pwBrokerRunning = false;
    bool pwPreviewRequested = false;
    bool pwActiveCasting = false;
    uint64_t pwFramesRelayed = 0;
    uint64_t pwFramesDropped = 0;
    uint32_t pwDmabufStreams = 0;
    uint32_t pwMemfdStreams = 0;
    uint32_t pwAudioRejects = 0;
    uint32_t pwCastLimitW = 0;
    uint32_t pwCastLimitH = 0;
    uint32_t pwCastLimitFps = 0;
    double pwActualFps = 0.0;
    QString pwSocketPath;
    QJsonArray pwClients;
};

class SalsaAutoGraphCanvas : public QWidget {
    Q_OBJECT
public:
    explicit SalsaAutoGraphCanvas(QWidget* parent = nullptr);

    QSize minimumSizeHint() const override { return QSize(620, 400); }
    QSize sizeHint() const override { return QSize(1060, 445); }

    void setTheme(const SalsaThemeData& theme, int fontPct = 100);
    void setSelectedNode(const QString& key);
    void updateData(const AutoGraphSnapshotData& data);
    void advancePulse(double dtSec);

signals:
    void nodeClicked(const QString& chapterKey);

protected:
    void mouseMoveEvent(QMouseEvent* event) override;
    void mousePressEvent(QMouseEvent* event) override;
    void leaveEvent(QEvent* event) override;
    void paintEvent(QPaintEvent* event) override;

private:
    struct HitNode {
        QString key;
        QString manualKey;
        QRectF rect;
    };

    void drawWire(
        QPainter& p,
        const QPointF& a,
        const QPointF& b,
        const QColor& col,
        bool dashed = false,
        bool pulse = true);
    void drawNode(
        QPainter& p,
        const QString& key,
        const QString& manualKey,
        const QRectF& r,
        const QString& title,
        const QString& badge,
        const QStringList& lines,
        const QColor& accent,
        bool hasIn,
        bool hasOut);

    SalsaThemeData theme_;
    int fontPct_ = 100;
    double pulsePhase_ = 0.0;
    QString hoveredNode_;
    QString selectedNode_;
    AutoGraphSnapshotData data_;
    std::vector<HitNode> hitNodes_;
};

class SalsaSpectrumCanvas : public QWidget {
    Q_OBJECT
public:
    explicit SalsaSpectrumCanvas(QWidget* parent = nullptr);

    QSize minimumSizeHint() const override { return QSize(480, 165); }
    QSize sizeHint() const override { return QSize(900, 210); }

    void setTheme(const SalsaThemeData& theme, int fontPct = 100);
    void updateTelemetry(
        bool enabled,
        uint32_t dynMode,
        double thresholdDb,
        const std::array<float, SALSA_EQ_BANDS>& gains,
        const std::array<float, SALSA_EQ_BANDS>& bandQ,
        const std::array<float, SALSA_EQ_BANDS>& bandFreqHz,
        const std::array<float, SALSA_EQ_BANDS>& deltas,
        const std::array<float, SALSA_EQ_BANDS>& levels);

signals:
    void bandGainDragged(int bandIdx, double newDb);
    void bandQChanged(int bandIdx, double newQ);
    void bandResetRequested(int bandIdx);

protected:
    void mousePressEvent(QMouseEvent* event) override;
    void mouseMoveEvent(QMouseEvent* event) override;
    void mouseReleaseEvent(QMouseEvent* event) override;
    void wheelEvent(QWheelEvent* event) override;
    void leaveEvent(QEvent* event) override;
    void paintEvent(QPaintEvent* event) override;

private:
    double bandX(int idx, double w) const;
    double gainToY(double db, double h) const;
    double yToGain(double y, double h) const;
    int hitBand(const QPointF& pos) const;

    SalsaThemeData theme_;
    int fontPct_ = 100;
    bool enabled_ = true;
    uint32_t dynMode_ = 1;
    double thresholdDb_ = -18.0;
    std::array<float, SALSA_EQ_BANDS> staticGains_{};
    std::array<float, SALSA_EQ_BANDS> bandQ_{};
    std::array<float, SALSA_EQ_BANDS> bandFreqHz_{};
    std::array<float, SALSA_EQ_BANDS> dynDeltas_{};
    std::array<float, SALSA_EQ_BANDS> smoothLevels_{};
    int draggingBand_ = -1;
    int hoverBand_ = -1;
};

class MiniSpectrumCanvas : public QWidget {
    Q_OBJECT
public:
    explicit MiniSpectrumCanvas(const SalsaThemeData& theme, QWidget* parent = nullptr);
    void setTheme(const SalsaThemeData& theme);

protected:
    void paintEvent(QPaintEvent* event) override;

private:
    SalsaThemeData theme_;
};

class MiniGraphCanvas : public QWidget {
    Q_OBJECT
public:
    explicit MiniGraphCanvas(const SalsaThemeData& theme, QWidget* parent = nullptr);
    void setTheme(const SalsaThemeData& theme);

protected:
    void paintEvent(QPaintEvent* event) override;

private:
    SalsaThemeData theme_;
};

class MiniSalsaPreview : public QFrame {
    Q_OBJECT
public:
    explicit MiniSalsaPreview(const SalsaThemeData& theme, QWidget* parent = nullptr);
    void setTheme(const SalsaThemeData& theme);

private:
    SalsaThemeData theme_;
    QTabWidget* tabs_ = nullptr;
    MiniSpectrumCanvas* spectrum_ = nullptr;
    MiniGraphCanvas* graph_ = nullptr;
};

class ColorSwatchRow : public QWidget {
    Q_OBJECT
public:
    ColorSwatchRow(
        const QString& key,
        const QString& label,
        const QString& hint,
        const QString& initialHex,
        QWidget* parent = nullptr);

    void setHex(const QString& hexColor);

signals:
    void colorChanged(const QString& key, const QString& hexColor);

private:
    void updateSwatchVisual();
    void onHexEdited(const QString& txt);
    void openPicker();

    QString key_;
    QString hex_;
    bool syncing_ = false;
    QPushButton* btnSwatch_ = nullptr;
    QLineEdit* txtHex_ = nullptr;
};

class SalsaCustomThemeDialog : public QDialog {
    Q_OBJECT
public:
    explicit SalsaCustomThemeDialog(
        const SalsaThemeData& initialTheme,
        int fontPct = 100,
        QWidget* parent = nullptr);

    SalsaThemeData resultTheme() const { return workingTheme_; }
    void resetForOpen(const SalsaThemeData& initialTheme, int fontPct = 100);

signals:
    void liveThemeChanged(const SalsaThemeData& theme);
    void mirrorStopped();
    void themeApplied(const SalsaThemeData& theme);

private:
    void buildUi();
    void refreshAllControlsFromTheme();
    void schedulePreviewUpdate();
    void flushPreviewUpdate();

    SalsaThemeData workingTheme_;
    SalsaThemeData initialSnapshot_;
    int fontPct_ = 100;
    QStringList presetNames_;
    QMap<QString, ColorSwatchRow*> swatchRows_;
    QComboBox* cmbClone_ = nullptr;
    QCheckBox* chkLight_ = nullptr;
    QCheckBox* chkMirrorMain_ = nullptr;
    QTabWidget* tokenTabs_ = nullptr;
    QLabel* lblSwatchStrip_ = nullptr;
    MiniSalsaPreview* miniPreview_ = nullptr;
    QTimer* previewTimer_ = nullptr;
};

class SalsaLiveDebuggerWidget : public QWidget {
    Q_OBJECT
public:
    explicit SalsaLiveDebuggerWidget(int initialTab = 0, QWidget* parent = nullptr);
    ~SalsaLiveDebuggerWidget() override;

    void setTheme(const SalsaThemeData& theme, int fontPct = 100);
    void selectTab(int idx);
    void refreshAll();
    void shutdown();

    bool isConnected() const { return connOk_; }
    QString connectionSummary() const { return connSummary_; }

signals:
    void statusUpdated(bool connected, const QString& summary);

private:
    struct DbgEventRow {
        uint64_t id = 0;
        int64_t tsMs = 0;
        QString sub;
        QString lvl;
        uint32_t pid = 0;
        uint32_t sid = 0;
        int32_t v1 = 0;
        int32_t v2 = 0;
        int32_t hr = 0;
        QString op;
        QString target;
        QString detail;
        QString jsonPretty;
    };

    void buildUi(int initialTab);
    QWidget* buildEventsTab();
    QWidget* buildProcTab();
    QWidget* buildTelemetryTab();
    QWidget* buildBoundaryTab();
    QWidget* buildScreenCastTab();

    void onPollTick();
    void rebuildEventTable();
    void renderProcTable();
    void renderTelemetryTab();
    void refreshBoundaryAudit();
    void renderConfigViewer();
    void onScreenCastTick();

    void onClearEvents();
    void sendPwPreviewCmd(bool enabled);
    void sendPwCastLimitsCmd();
    void saveScreenCastPrefs();
    void updateScProfileBadge(double measuredFps = 0.0, const QString& activeDims = QString());

    void* dbg_ = nullptr;
    void* shmCtx_ = nullptr;
    SalsaThemeData theme_;
    int fontPct_ = 100;
    bool connOk_ = false;
    QString connSummary_;
    bool paused_ = false;
    uint64_t clearWatermarkId_ = 0;
    qint64 clearExtAfterMs_ = 0;

    QJsonObject latestStats_;
    QJsonObject latestSnapshot_;
    QJsonObject latestAudit_;
    QJsonArray latestProcs_;
    std::vector<DbgEventRow> allEvents_;
    std::vector<DbgEventRow> displayedEvents_;
    std::vector<uint8_t> scRgbaBuf_;
    QImage lastScImage_;
    uint32_t lastScW_ = 0;
    uint32_t lastScH_ = 0;
    uint64_t lastShmSeq_ = 0;
    QList<qint64> scFrameTimesMs_;
    bool scLimitsDirty_ = false;

    QTimer* pollTimer_ = nullptr;
    QTimer* scTimer_ = nullptr;

    // Header / health controls
    QLabel* lblConn_ = nullptr;
    QLabel* lblRtPill_ = nullptr;
    QPushButton* btnPause_ = nullptr;
    QPushButton* btnResetNet_ = nullptr;
    QTabWidget* subtabs_ = nullptr;

    // Tab 1: Live Event Stream
    QComboBox* cmbSub_ = nullptr;
    QComboBox* cmbLvl_ = nullptr;
    QLineEdit* txtSearch_ = nullptr;
    QCheckBox* chkAutoscroll_ = nullptr;
    QCheckBox* chkJournal_ = nullptr;
    QCheckBox* chkAutosync_ = nullptr;
    QCheckBox* chkDbus_ = nullptr;
    QTableWidget* tblEvents_ = nullptr;
    QPlainTextEdit* txtEventDetail_ = nullptr;

    // Tab 2: Process Audit
    QComboBox* cmbProcFilter_ = nullptr;
    QLineEdit* txtProcSearch_ = nullptr;
    QTableWidget* tblProcs_ = nullptr;
    QPlainTextEdit* txtProcDetail_ = nullptr;

    // Tab 3: Telemetry & Rings
    QLabel* lblEngSummary_ = nullptr;
    QLabel* lblBtSummary_ = nullptr;
    QLabel* lblCapMidiSummary_ = nullptr;
    QProgressBar* barRtLoad_ = nullptr;
    QTableWidget* tblWasapi_ = nullptr;
    QTableWidget* tblPaClients_ = nullptr;
    QTableWidget* tblPaStreams_ = nullptr;

    // Tab 4: Boundary & Probes
    QLabel* lblStaleWarn_ = nullptr;
    QPlainTextEdit* txtProbeOut_ = nullptr;
    QTableWidget* tblUnits_ = nullptr;
    QTableWidget* tblSockets_ = nullptr;
    QComboBox* cmbCfg_ = nullptr;
    QPlainTextEdit* txtCfgView_ = nullptr;

    // Tab 5: ScreenCast
    QComboBox* cmbScMode_ = nullptr;
    QComboBox* cmbScRes_ = nullptr;
    QComboBox* cmbScFps_ = nullptr;
    QCheckBox* chkScCapBroker_ = nullptr;
    QPushButton* btnScLive_ = nullptr;
    QPushButton* btnScSaveFrame_ = nullptr;
    QLabel* lblScProfileBadge_ = nullptr;
    QLabel* lblScStats_ = nullptr;
    QLabel* lblScDmabufBadge_ = nullptr;
    QLabel* lblScPreview_ = nullptr;
};

}  // namespace salsa

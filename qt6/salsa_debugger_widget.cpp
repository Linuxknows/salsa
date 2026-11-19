#include "salsa_widgets.hpp"

#include <QAbstractItemView>
#include <QDateTime>
#include <QFont>
#include <QGridLayout>
#include <QGuiApplication>
#include <QHBoxLayout>
#include <QHeaderView>
#include <QMessageBox>
#include <QPixmap>
#include <QScreen>
#include <QScrollBar>
#include <QSplitter>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>

namespace salsa {

namespace {

class NumericTableItem : public QTableWidgetItem {
public:
    explicit NumericTableItem(qint64 num) : QTableWidgetItem(QString::number(num)), val_(num) {
        setData(Qt::EditRole, QVariant::fromValue(num));
    }
    bool operator<(const QTableWidgetItem& other) const override {
        const auto* rhs = dynamic_cast<const NumericTableItem*>(&other);
        if (rhs) return val_ < rhs->val_;
        return QTableWidgetItem::operator<(other);
    }

private:
    qint64 val_;
};

QTableWidgetItem* makeRoItem(const QString& txt) {
    auto* it = new QTableWidgetItem(txt);
    it->setFlags(it->flags() & ~Qt::ItemIsEditable);
    return it;
}

}  // namespace

SalsaLiveDebuggerWidget::SalsaLiveDebuggerWidget(int initialTab, QWidget* parent)
    : QWidget(parent) {
    dbg_ = salsa_dbg_create();
    shmCtx_ = salsa_dbg_shm_create();

    buildUi(initialTab);

    pollTimer_ = new QTimer(this);
    pollTimer_->setInterval(350);
    connect(pollTimer_, &QTimer::timeout, this, &SalsaLiveDebuggerWidget::onPollTick);
    pollTimer_->start();

    scTimer_ = new QTimer(this);
    scTimer_->setInterval(33);
    connect(scTimer_, &QTimer::timeout, this, &SalsaLiveDebuggerWidget::onScreenCastTick);

    refreshAll();
}

SalsaLiveDebuggerWidget::~SalsaLiveDebuggerWidget() {
    shutdown();
}

void SalsaLiveDebuggerWidget::shutdown() {
    if (scTimer_) scTimer_->stop();
    if (pollTimer_) pollTimer_->stop();
    if (shmCtx_) {
        salsa_dbg_shm_stop(shmCtx_);
        salsa_dbg_shm_destroy(shmCtx_);
        shmCtx_ = nullptr;
    }
    if (dbg_) {
        if (btnScLive_ && btnScLive_->isChecked()) {
            salsa_dbg_send_command(dbg_, "PW_PREVIEW 0");
        }
        if (scLimitsDirty_) {
            const QJsonObject prefs = parseJsonObj(SalsaStr(salsa_dbg_load_sc_prefs_json()).toQString());
            if (!prefs.value(QStringLiteral("cap_real_cast")).toBool(false)) {
                salsa_dbg_send_command(dbg_, "PW_CAST_LIMITS 0 0 0");
            }
        }
        salsa_dbg_destroy(dbg_);
        dbg_ = nullptr;
    }
}

void SalsaLiveDebuggerWidget::selectTab(int idx) {
    if (subtabs_ && idx >= 0 && idx < subtabs_->count()) {
        subtabs_->setCurrentIndex(idx);
    }
}

void SalsaLiveDebuggerWidget::refreshAll() {
    if (dbg_) salsa_dbg_request_snapshot(dbg_);
    onPollTick();
    refreshBoundaryAudit();
}

void SalsaLiveDebuggerWidget::setTheme(const SalsaThemeData& theme, int fontPct) {
    theme_ = theme;
    fontPct_ = normalizeFontScalePct(fontPct);

    const int fs = scaledPx(11, fontPct_);
    const int hdrFs = scaledPx(12, fontPct_);
    const QString bgMain = theme_.color("bg_main", "#111316");
    const QString bgCard = theme_.color("bg_card", "#181b20");
    const QString bgDeep = theme_.color("bg_deep", "#0c0e11");
    const QString bdCard = theme_.color("border_card", "#262b33");
    const QString bdCtrl = theme_.color("border_control", "#334155");
    const QString txtPri = theme_.color("text_primary", "#e2e8f0");
    const QString txtSec = theme_.color("text_secondary", "#cbd5e1");
    const QString txtBrt = theme_.color("text_bright", "#f8fafc");
    const QString accPri = theme_.color("accent_primary", "#00e676");
    const QString selBg = theme_.color("combo_popup_sel_bg", "#1e3a2f");

    if (lblConn_) {
        QFont cf(QStringLiteral("Monospace"));
        cf.setPixelSize(scaledPx(11, fontPct_));
        cf.setWeight(QFont::Bold);
        lblConn_->setFont(cf);
    }
    const int rowH = scaledPx(24, fontPct_);
    for (QTableWidget* tbl : {tblEvents_, tblProcs_, tblWasapi_, tblPaClients_, tblPaStreams_, tblUnits_, tblSockets_}) {
        if (tbl) tbl->verticalHeader()->setDefaultSectionSize(rowH);
    }

    setStyleSheet(QStringLiteral(
                      "QWidget#DbgRoot { background-color: %1; color: %2; font-size: %3px; }"
                      " QWidget#DbgRoot QLabel, QWidget#DbgRoot QCheckBox, QWidget#DbgRoot QPushButton, QWidget#DbgRoot QComboBox { font-size: %3px; }"
                      " QFrame#DbgCard { background-color: %4; border: 1px solid %5; border-radius: 8px; }"
                      " QLabel#DbgHeader { font-size: %6px; font-weight: 700; color: %7; }"
                      " QLabel#DbgStat { font-family: Monospace; font-size: %3px; color: %8; }"
                      " QLineEdit, QPlainTextEdit { background-color: %9; color: %2; border: 1px solid %10;"
                      " border-radius: 6px; padding: 4px 8px; font-family: Monospace; font-size: %3px; }"
                      " QTableWidget { background-color: %9; color: %2; gridline-color: %5; border: 1px solid %5;"
                      " border-radius: 6px; selection-background-color: %11; selection-color: %12; font-size: %3px; }"
                      " QHeaderView::section, QTableCornerButton::section { background-color: %4; color: %8;"
                      " border: 1px solid %5; padding: 3px 6px; font-weight: 700; font-size: %3px; }")
                      .arg(bgMain, txtPri, QString::number(fs), bgCard, bdCard, QString::number(hdrFs),
                           accPri, txtSec, bgDeep, bdCtrl, selBg, txtBrt));
}

void SalsaLiveDebuggerWidget::buildUi(int initialTab) {
    setObjectName(QStringLiteral("DbgRoot"));
    auto* root = new QVBoxLayout(this);
    root->setContentsMargins(8, 8, 8, 8);
    root->setSpacing(6);

    auto* topCard = new QFrame(this);
    topCard->setObjectName(QStringLiteral("DbgCard"));
    auto* topCol = new QVBoxLayout(topCard);
    topCol->setContentsMargins(10, 8, 10, 8);
    topCol->setSpacing(6);

    auto* row1 = new QHBoxLayout();
    lblConn_ = new QLabel(QStringLiteral("CONNECTING debug.sock ..."), topCard);
    lblConn_->setFont(QFont(QStringLiteral("Monospace"), scaledPx(10, fontPct_), QFont::Bold));
    row1->addWidget(lblConn_);

    lblRtPill_ = new QLabel(QStringLiteral("RT: waiting..."), topCard);
    lblRtPill_->setObjectName(QStringLiteral("DbgStat"));
    lblRtPill_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    lblRtPill_->setMinimumWidth(0);
    row1->addWidget(lblRtPill_, 1);
    topCol->addLayout(row1);

    auto* row2 = new QHBoxLayout();
    row2->setSpacing(6);
    row2->addStretch(1);

    btnPause_ = new QPushButton(QStringLiteral("Pause Stream"), topCard);
    btnPause_->setCheckable(true);
    connect(btnPause_, &QPushButton::toggled, this, [this](bool on) {
        paused_ = on;
        btnPause_->setText(on ? QStringLiteral("Resume Stream") : QStringLiteral("Pause Stream"));
    });
    row2->addWidget(btnPause_);

    auto* btnClear = new QPushButton(QStringLiteral("Clear Events"), topCard);
    connect(btnClear, &QPushButton::clicked, this, &SalsaLiveDebuggerWidget::onClearEvents);
    row2->addWidget(btnClear);

    auto* btnResetRt = new QPushButton(QStringLiteral("Reset RT Stats"), topCard);
    connect(btnResetRt, &QPushButton::clicked, this, [this]() {
        if (dbg_) {
            salsa_dbg_send_command(dbg_, "RESET_STATS");
            salsa_dbg_request_snapshot(dbg_);
        }
    });
    row2->addWidget(btnResetRt);

    auto* btnSnap = new QPushButton(QStringLiteral("Snapshot Now"), topCard);
    connect(btnSnap, &QPushButton::clicked, this, [this]() {
        if (dbg_) salsa_dbg_request_snapshot(dbg_);
    });
    row2->addWidget(btnSnap);

    auto* btnExport = new QPushButton(QStringLiteral("Export Bundle"), topCard);
    connect(btnExport, &QPushButton::clicked, this, [this]() {
        const SalsaStr msg(salsa_dbg_export_bundle(dbg_));
        if (txtProbeOut_) txtProbeOut_->setPlainText(msg.toQString());
        lblRtPill_->setText(msg.toQString());
    });
    row2->addWidget(btnExport);

    btnResetNet_ = new QPushButton(QStringLiteral("Reset PW / Video Clients"), topCard);
    btnResetNet_->setToolTip(QStringLiteral("Safely disconnects stale PipeWire ScreenCast portal clients"));
    connect(btnResetNet_, &QPushButton::clicked, this, [this]() {
        if (dbg_) {
            const SalsaStr res(salsa_dbg_reset_net_clients_json(dbg_));
            if (txtProbeOut_) txtProbeOut_->setPlainText(QStringLiteral("Reset PW/Video Clients: %1").arg(res.toQString()));
            salsa_dbg_request_snapshot(dbg_);
        }
    });
    row2->addWidget(btnResetNet_);
    topCol->addLayout(row2);
    root->addWidget(topCard);

    subtabs_ = new QTabWidget(this);
    subtabs_->addTab(buildEventsTab(), QStringLiteral("Live Event Stream"));
    subtabs_->addTab(buildProcTab(), QStringLiteral("App && Process Routing Audit"));
    subtabs_->addTab(buildTelemetryTab(), QStringLiteral("Streams, Rings && RT Timing"));
    subtabs_->addTab(buildBoundaryTab(), QStringLiteral("Boundary, Configs && Probes"));
    subtabs_->addTab(buildScreenCastTab(), QStringLiteral("ScreenCast Broker && Live Feed"));
    subtabs_->setCurrentIndex(std::clamp(initialTab, 0, subtabs_->count() - 1));
    root->addWidget(subtabs_, 1);
}

QWidget* SalsaLiveDebuggerWidget::buildEventsTab() {
    auto* w = new QWidget(this);
    auto* lay = new QVBoxLayout(w);
    lay->setContentsMargins(6, 6, 6, 6);
    lay->setSpacing(6);

    auto* fbar = new QHBoxLayout();
    fbar->addWidget(new QLabel(QStringLiteral("Subsystem:"), w));
    cmbSub_ = new QComboBox(w);
    cmbSub_->addItem(QStringLiteral("ALL"));
    const QJsonArray subs = parseJsonArr(SalsaStr(salsa_dbg_subsystems_json()).toQString());
    for (const QJsonValue& v : subs) cmbSub_->addItem(v.toString());
    connect(cmbSub_, &QComboBox::currentTextChanged, this, [this]() { rebuildEventTable(); });
    fbar->addWidget(cmbSub_);

    fbar->addWidget(new QLabel(QStringLiteral("Min Level:"), w));
    cmbLvl_ = new QComboBox(w);
    const QJsonArray lvls = parseJsonArr(SalsaStr(salsa_dbg_log_levels_json()).toQString());
    for (const QJsonValue& v : lvls) cmbLvl_->addItem(v.toString());
    cmbLvl_->setCurrentText(QStringLiteral("TRACE"));
    connect(cmbLvl_, &QComboBox::currentTextChanged, this, [this]() { rebuildEventTable(); });
    fbar->addWidget(cmbLvl_);

    txtSearch_ = new QLineEdit(w);
    txtSearch_->setPlaceholderText(QStringLiteral("Filter by PID, op, target, or detail..."));
    connect(txtSearch_, &QLineEdit::textChanged, this, [this]() { rebuildEventTable(); });
    fbar->addWidget(txtSearch_, 1);

    chkAutoscroll_ = new QCheckBox(QStringLiteral("Auto-Scroll"), w);
    chkAutoscroll_->setChecked(true);
    fbar->addWidget(chkAutoscroll_);

    chkJournal_ = new QCheckBox(QStringLiteral("+ journalctl"), w);
    connect(chkJournal_, &QCheckBox::toggled, this, [this]() { onPollTick(); });
    fbar->addWidget(chkJournal_);

    chkAutosync_ = new QCheckBox(QStringLiteral("+ autosync.log"), w);
    connect(chkAutosync_, &QCheckBox::toggled, this, [this]() { onPollTick(); });
    fbar->addWidget(chkAutosync_);

    chkDbus_ = new QCheckBox(QStringLiteral("+ BlueZ D-Bus"), w);
    connect(chkDbus_, &QCheckBox::toggled, this, [this]() { onPollTick(); });
    fbar->addWidget(chkDbus_);
    lay->addLayout(fbar);

    auto* split = new QSplitter(Qt::Vertical, w);
    tblEvents_ = new QTableWidget(0, 9, split);
    tblEvents_->setHorizontalHeaderLabels({
        QStringLiteral("Time"), QStringLiteral("Sub"), QStringLiteral("Lvl"),
        QStringLiteral("Operation"), QStringLiteral("Target"), QStringLiteral("PID"),
        QStringLiteral("SID"), QStringLiteral("Vals / HR"), QStringLiteral("Detail")});
    tblEvents_->horizontalHeader()->setSectionResizeMode(8, QHeaderView::Stretch);
    const int widths[8] = {95, 76, 56, 130, 150, 62, 48, 115};
    for (int i = 0; i < 8; ++i) tblEvents_->setColumnWidth(i, widths[i]);
    tblEvents_->verticalHeader()->setVisible(false);
    tblEvents_->setEditTriggers(QAbstractItemView::NoEditTriggers);
    tblEvents_->setSelectionBehavior(QAbstractItemView::SelectRows);
    connect(tblEvents_, &QTableWidget::itemSelectionChanged, this, [this]() {
        const int r = tblEvents_->currentRow();
        if (r >= 0 && r < static_cast<int>(displayedEvents_.size())) {
            txtEventDetail_->setPlainText(displayedEvents_[static_cast<size_t>(r)].jsonPretty);
        }
    });
    split->addWidget(tblEvents_);

    txtEventDetail_ = new QPlainTextEdit(split);
    txtEventDetail_->setReadOnly(true);
    txtEventDetail_->setMaximumHeight(135);
    txtEventDetail_->setPlaceholderText(QStringLiteral("Click any event row to inspect its full JSON payload..."));
    split->addWidget(txtEventDetail_);
    split->setStretchFactor(0, 4);
    split->setStretchFactor(1, 1);
    lay->addWidget(split, 1);
    return w;
}

QWidget* SalsaLiveDebuggerWidget::buildProcTab() {
    auto* w = new QWidget(this);
    auto* lay = new QVBoxLayout(w);
    lay->setContentsMargins(6, 6, 6, 6);
    lay->setSpacing(6);

    auto* bar = new QHBoxLayout();
    bar->addWidget(new QLabel(QStringLiteral("Routing Filter:"), w));
    cmbProcFilter_ = new QComboBox(w);
    cmbProcFilter_->addItems({
        QStringLiteral("All Audio Processes"),
        QStringLiteral("Leaks / Direct ALSA Only"),
        QStringLiteral("Salsa Routed Only")});
    connect(cmbProcFilter_, &QComboBox::currentTextChanged, this, [this]() { renderProcTable(); });
    bar->addWidget(cmbProcFilter_);

    txtProcSearch_ = new QLineEdit(w);
    txtProcSearch_->setPlaceholderText(QStringLiteral("Search process name, PID, or .so path..."));
    connect(txtProcSearch_, &QLineEdit::textChanged, this, [this]() { renderProcTable(); });
    bar->addWidget(txtProcSearch_, 1);

    auto* btnRef = new QPushButton(QStringLiteral("Refresh Process Audit"), w);
    btnRef->setObjectName(QStringLiteral("PrimaryBtn"));
    connect(btnRef, &QPushButton::clicked, this, [this]() {
        latestProcs_ = parseJsonArr(SalsaStr(salsa_dbg_scan_procs_json()).toQString());
        renderProcTable();
    });
    bar->addWidget(btnRef);
    lay->addLayout(bar);

    auto* split = new QSplitter(Qt::Vertical, w);
    tblProcs_ = new QTableWidget(0, 6, split);
    tblProcs_->setHorizontalHeaderLabels({
        QStringLiteral("PID"), QStringLiteral("Comm"), QStringLiteral("Classification"),
        QStringLiteral("Shm / Sockets"), QStringLiteral("ALSA FDs"), QStringLiteral("Mapped Libs / Detail")});
    tblProcs_->horizontalHeader()->setSectionResizeMode(5, QHeaderView::Stretch);
    tblProcs_->verticalHeader()->setVisible(false);
    tblProcs_->setEditTriggers(QAbstractItemView::NoEditTriggers);
    tblProcs_->setSelectionBehavior(QAbstractItemView::SelectRows);
    tblProcs_->setSortingEnabled(true);
    connect(tblProcs_, &QTableWidget::itemSelectionChanged, this, [this]() {
        const int row = tblProcs_->currentRow();
        if (row < 0) return;
        const auto* itemPid = tblProcs_->item(row, 0);
        if (!itemPid) return;
        const qint64 pid = itemPid->text().toLongLong();
        for (const QJsonValue& pv : latestProcs_) {
            const QJsonObject obj = pv.toObject();
            if (obj.value(QStringLiteral("pid")).toInteger() == pid) {
                txtProcDetail_->setPlainText(QString::fromUtf8(QJsonDocument(obj).toJson(QJsonDocument::Indented)));
                break;
            }
        }
    });
    split->addWidget(tblProcs_);

    txtProcDetail_ = new QPlainTextEdit(split);
    txtProcDetail_->setReadOnly(true);
    txtProcDetail_->setMaximumHeight(130);
    txtProcDetail_->setPlaceholderText(QStringLiteral("Select a process row to inspect cmdline, ALSA FDs, and loaded audio shared libraries..."));
    split->addWidget(txtProcDetail_);
    split->setStretchFactor(0, 4);
    split->setStretchFactor(1, 1);
    lay->addWidget(split, 1);
    return w;
}

QWidget* SalsaLiveDebuggerWidget::buildTelemetryTab() {
    auto* w = new QWidget(this);
    auto* lay = new QVBoxLayout(w);
    lay->setContentsMargins(6, 6, 6, 6);
    lay->setSpacing(6);

    auto* topCards = new QHBoxLayout();
    auto makeCard = [&](const QString& hdrText, QLabel** outLbl, QProgressBar** outBar = nullptr) {
        auto* card = new QFrame(w);
        card->setObjectName(QStringLiteral("DbgCard"));
        card->setMinimumWidth(0);
        auto* cl = new QVBoxLayout(card);
        cl->setContentsMargins(10, 8, 10, 8);
        auto* hdr = new QLabel(hdrText, card);
        hdr->setObjectName(QStringLiteral("DbgHeader"));
        hdr->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
        hdr->setWordWrap(true);
        cl->addWidget(hdr);
        auto* lbl = new QLabel(QStringLiteral("Waiting for snapshot..."), card);
        lbl->setObjectName(QStringLiteral("DbgStat"));
        lbl->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
        lbl->setWordWrap(true);
        cl->addWidget(lbl);
        if (outBar) {
            auto* bar = new QProgressBar(card);
            bar->setRange(0, 1000);
            bar->setValue(0);
            bar->setFormat(QStringLiteral("0.00% RT CPU"));
            cl->addWidget(bar);
            *outBar = bar;
        }
        *outLbl = lbl;
        topCards->addWidget(card, 1);
    };

    makeCard(QStringLiteral("ENGINE CLOCK & RT TIMING"), &lblEngSummary_, &barRtLoad_);
    makeCard(QStringLiteral("BLUEZ A2DP TX RING & TRANSPORT"), &lblBtSummary_);
    makeCard(QStringLiteral("CAPTURE DUPLEX & DAW MIDI HUB"), &lblCapMidiSummary_);
    lay->addLayout(topCards);

    tblWasapi_ = new QTableWidget(0, 8, w);
    tblWasapi_->setHorizontalHeaderLabels({
        QStringLiteral("SID"), QStringLiteral("PID"), QStringLiteral("Client"),
        QStringLiteral("Dir"), QStringLiteral("Mode"), QStringLiteral("Rate / Ch"),
        QStringLiteral("Buffered"), QStringLiteral("XRUNs / Vol")});
    tblWasapi_->horizontalHeader()->setSectionResizeMode(2, QHeaderView::Stretch);
    tblWasapi_->verticalHeader()->setVisible(false);
    tblWasapi_->setEditTriggers(QAbstractItemView::NoEditTriggers);
    lay->addWidget(tblWasapi_, 1);

    auto* paSplit = new QSplitter(Qt::Horizontal, w);
    tblPaClients_ = new QTableWidget(0, 5, paSplit);
    tblPaClients_->setHorizontalHeaderLabels({
        QStringLiteral("Idx"), QStringLiteral("PID"), QStringLiteral("App"),
        QStringLiteral("Proto"), QStringLiteral("Out")});
    auto* pcHdr = tblPaClients_->horizontalHeader();
    pcHdr->setMinimumSectionSize(32);
    pcHdr->setSectionResizeMode(0, QHeaderView::ResizeToContents);
    pcHdr->setSectionResizeMode(1, QHeaderView::ResizeToContents);
    pcHdr->setSectionResizeMode(2, QHeaderView::Stretch);
    pcHdr->setSectionResizeMode(3, QHeaderView::ResizeToContents);
    pcHdr->setSectionResizeMode(4, QHeaderView::ResizeToContents);
    tblPaClients_->verticalHeader()->setVisible(false);
    tblPaClients_->setEditTriggers(QAbstractItemView::NoEditTriggers);
    paSplit->addWidget(tblPaClients_);

    tblPaStreams_ = new QTableWidget(0, 7, paSplit);
    tblPaStreams_->setHorizontalHeaderLabels({
        QStringLiteral("Idx"), QStringLiteral("PID"), QStringLiteral("Stream"),
        QStringLiteral("Type"), QStringLiteral("Fmt"), QStringLiteral("Fill / Cap"), QStringLiteral("Peak / Vol")});
    auto* psHdr = tblPaStreams_->horizontalHeader();
    psHdr->setMinimumSectionSize(32);
    psHdr->setSectionResizeMode(0, QHeaderView::ResizeToContents);
    psHdr->setSectionResizeMode(1, QHeaderView::ResizeToContents);
    psHdr->setSectionResizeMode(2, QHeaderView::Stretch);
    psHdr->setSectionResizeMode(3, QHeaderView::ResizeToContents);
    psHdr->setSectionResizeMode(4, QHeaderView::ResizeToContents);
    psHdr->setSectionResizeMode(5, QHeaderView::ResizeToContents);
    psHdr->setSectionResizeMode(6, QHeaderView::ResizeToContents);
    tblPaStreams_->verticalHeader()->setVisible(false);
    tblPaStreams_->setEditTriggers(QAbstractItemView::NoEditTriggers);
    paSplit->addWidget(tblPaStreams_);
    paSplit->setStretchFactor(0, 2);
    paSplit->setStretchFactor(1, 3);
    lay->addWidget(paSplit, 1);
    return w;
}

QWidget* SalsaLiveDebuggerWidget::buildBoundaryTab() {
    auto* w = new QWidget(this);
    auto* lay = new QVBoxLayout(w);
    lay->setContentsMargins(6, 6, 6, 6);
    lay->setSpacing(6);

    auto* probeCard = new QFrame(w);
    probeCard->setObjectName(QStringLiteral("DbgCard"));
    auto* pCol = new QVBoxLayout(probeCard);
    pCol->setContentsMargins(10, 8, 10, 8);
    pCol->setSpacing(6);

    auto* prow = new QHBoxLayout();
    auto* hdrP = new QLabel(QStringLiteral("INTERACTIVE SYSTEM & HARDWARE PROBES"), probeCard);
    hdrP->setObjectName(QStringLiteral("DbgHeader"));
    prow->addWidget(hdrP);
    prow->addStretch(1);

    auto* bPing = new QPushButton(QStringLiteral("IPC + Debug Ping RTT"), probeCard);
    connect(bPing, &QPushButton::clicked, this, [this]() {
        const QJsonObject pong = parseJsonObj(SalsaStr(salsa_dbg_ping_json(dbg_, "ui")).toQString());
        if (pong.value(QStringLiteral("ok")).toBool()) {
            txtProbeOut_->setPlainText(
                QStringLiteral("Debug PING RTT: %1 ms\n%2")
                    .arg(pong.value(QStringLiteral("rtt_ms")).toDouble(), 0, 'f', 2)
                    .arg(pong.value(QStringLiteral("pretty")).toString()));
        } else {
            txtProbeOut_->setPlainText(QStringLiteral("Debug PING timed out (audiodg unreachable or busy)."));
        }
    });
    prow->addWidget(bPing);

    auto* bTone = new QPushButton(QStringLiteral("Play 440Hz WASAPI Probe Tone"), probeCard);
    connect(bTone, &QPushButton::clicked, this, [this]() {
        txtProbeOut_->setPlainText(SalsaStr(salsa_dbg_play_tone()).toQString());
    });
    prow->addWidget(bTone);

    auto* bMidi = new QPushButton(QStringLiteral("Send MIDI Note Ping (C4)"), probeCard);
    connect(bMidi, &QPushButton::clicked, this, [this]() {
        txtProbeOut_->setPlainText(SalsaStr(salsa_dbg_send_midi()).toQString());
    });
    prow->addWidget(bMidi);

    auto* bAllProbes = new QPushButton(QStringLiteral("Run All Socket && PA/PW Probes"), probeCard);
    connect(bAllProbes, &QPushButton::clicked, this, [this]() {
        txtProbeOut_->setPlainText(SalsaStr(salsa_dbg_run_probes_json()).toQString());
    });
    prow->addWidget(bAllProbes);

    auto* bRef = new QPushButton(QStringLiteral("Refresh Boundary Audit"), probeCard);
    bRef->setObjectName(QStringLiteral("PrimaryBtn"));
    connect(bRef, &QPushButton::clicked, this, &SalsaLiveDebuggerWidget::refreshBoundaryAudit);
    prow->addWidget(bRef);
    pCol->addLayout(prow);

    lblStaleWarn_ = new QLabel(QStringLiteral("Build & Binary Freshness: checking..."), probeCard);
    lblStaleWarn_->setObjectName(QStringLiteral("DbgStat"));
    pCol->addWidget(lblStaleWarn_);

    txtProbeOut_ = new QPlainTextEdit(probeCard);
    txtProbeOut_->setReadOnly(true);
    txtProbeOut_->setMaximumHeight(82);
    txtProbeOut_->setPlaceholderText(QStringLiteral("Click any interactive probe button above to test IPC, WASAPI PCM audio, MIDI injection, or PA/PW sockets..."));
    pCol->addWidget(txtProbeOut_);
    lay->addWidget(probeCard);

    auto* split = new QSplitter(Qt::Horizontal, w);
    auto* leftW = new QWidget(split);
    auto* leftLay = new QVBoxLayout(leftW);
    leftLay->setContentsMargins(0, 0, 0, 0);

    tblUnits_ = new QTableWidget(0, 5, leftW);
    tblUnits_->setHorizontalHeaderLabels({
        QStringLiteral("Unit"), QStringLiteral("Active"), QStringLiteral("Sub"),
        QStringLiteral("Enabled"), QStringLiteral("MainPID")});
    tblUnits_->horizontalHeader()->setSectionResizeMode(0, QHeaderView::Stretch);
    tblUnits_->verticalHeader()->setVisible(false);
    tblUnits_->setEditTriggers(QAbstractItemView::NoEditTriggers);
    leftLay->addWidget(tblUnits_, 3);

    tblSockets_ = new QTableWidget(0, 5, leftW);
    tblSockets_->setHorizontalHeaderLabels({
        QStringLiteral("Boundary Socket"), QStringLiteral("Status"), QStringLiteral("Peer PID"),
        QStringLiteral("Owner Comm"), QStringLiteral("Path")});
    tblSockets_->horizontalHeader()->setSectionResizeMode(4, QHeaderView::Stretch);
    tblSockets_->verticalHeader()->setVisible(false);
    tblSockets_->setEditTriggers(QAbstractItemView::NoEditTriggers);
    leftLay->addWidget(tblSockets_, 2);
    split->addWidget(leftW);

    auto* rightW = new QWidget(split);
    auto* rightLay = new QVBoxLayout(rightW);
    rightLay->setContentsMargins(0, 0, 0, 0);
    auto* cfgBar = new QHBoxLayout();
    cfgBar->addWidget(new QLabel(QStringLiteral("Inspect Config / Runtime:"), rightW));
    cmbCfg_ = new QComboBox(rightW);
    cmbCfg_->addItems({
        QStringLiteral("~/.asoundrc"),
        QStringLiteral("~/.config/OpenAL/alsoft.conf"),
        QStringLiteral("pipewire_zero_mode"),
        QStringLiteral("99-salsa-direct.conf"),
        QStringLiteral("99-lwasapi-video-only.conf"),
        QStringLiteral("module_versions.state"),
        QStringLiteral("src/alsa.version"),
        QStringLiteral("src/pulse.version"),
        QStringLiteral("ALSA /proc/asound + Build Freshness"),
    });
    connect(cmbCfg_, &QComboBox::currentTextChanged, this, [this]() { renderConfigViewer(); });
    cfgBar->addWidget(cmbCfg_, 1);
    rightLay->addLayout(cfgBar);

    txtCfgView_ = new QPlainTextEdit(rightW);
    txtCfgView_->setReadOnly(true);
    rightLay->addWidget(txtCfgView_, 1);
    split->addWidget(rightW);

    split->setStretchFactor(0, 3);
    split->setStretchFactor(1, 2);
    lay->addWidget(split, 1);
    return w;
}

QWidget* SalsaLiveDebuggerWidget::buildScreenCastTab() {
    auto* w = new QWidget(this);
    auto* lay = new QVBoxLayout(w);
    lay->setContentsMargins(6, 6, 6, 6);
    lay->setSpacing(8);

    const QJsonObject prefs = parseJsonObj(SalsaStr(salsa_dbg_load_sc_prefs_json()).toQString());
    const QString savedRes = prefs.value(QStringLiteral("res_key")).toString(QStringLiteral("Source"));
    const int savedFps = prefs.value(QStringLiteral("fps_val")).toInt(60);
    const bool savedCap = prefs.value(QStringLiteral("cap_real_cast")).toBool(false);

    auto* ctrlCard = new QFrame(w);
    ctrlCard->setObjectName(QStringLiteral("DbgCard"));
    auto* ctrlCol = new QVBoxLayout(ctrlCard);
    ctrlCol->setContentsMargins(10, 8, 10, 8);
    ctrlCol->setSpacing(6);

    auto* r1 = new QHBoxLayout();
    auto* hdr = new QLabel(QStringLiteral("ZERO-PIPEWIRE SCREENCAST BROKER & SHM VIDEO PREVIEW"), ctrlCard);
    hdr->setObjectName(QStringLiteral("DbgHeader"));
    r1->addWidget(hdr);
    r1->addStretch(1);

    r1->addWidget(new QLabel(QStringLiteral("Mode:"), ctrlCard));
    cmbScMode_ = new QComboBox(ctrlCard);
    cmbScMode_->addItems({
        QStringLiteral("Auto (SHM if Casting else Dry-Run)"),
        QStringLiteral("SHM Preview (Zero-Overhead Live)"),
        QStringLiteral("Dry-Run Portal Sim (Isolated)"),
    });
    r1->addWidget(cmbScMode_);

    r1->addWidget(new QLabel(QStringLiteral("Res:"), ctrlCard));
    cmbScRes_ = new QComboBox(ctrlCard);
    struct ResSpec { const char* key; const char* lbl; uint32_t w; uint32_t h; };
    const ResSpec resSpecs[] = {
        {"Source", "Source (Native)", 0, 0},
        {"2160p", "2160p (4K)", 3840, 2160},
        {"1440p", "1440p (QHD)", 2560, 1440},
        {"1080p", "1080p (FHD)", 1920, 1080},
        {"720p", "720p (HD)", 1280, 720},
        {"480p", "480p (SD)", 854, 480},
    };
    for (const auto& rs : resSpecs) {
        QJsonObject d;
        d.insert(QStringLiteral("key"), QString::fromLatin1(rs.key));
        d.insert(QStringLiteral("w"), static_cast<qint64>(rs.w));
        d.insert(QStringLiteral("h"), static_cast<qint64>(rs.h));
        cmbScRes_->addItem(QString::fromLatin1(rs.lbl), jsonCompact(d));
        if (savedRes == QLatin1String(rs.key)) cmbScRes_->setCurrentIndex(cmbScRes_->count() - 1);
    }
    connect(cmbScRes_, &QComboBox::currentIndexChanged, this, [this]() {
        saveScreenCastPrefs();
        if (chkScCapBroker_ && chkScCapBroker_->isChecked()) sendPwCastLimitsCmd();
        if (btnScLive_ && btnScLive_->isChecked()) sendPwPreviewCmd(true);
        updateScProfileBadge();
    });
    r1->addWidget(cmbScRes_);

    r1->addWidget(new QLabel(QStringLiteral("FPS:"), ctrlCard));
    cmbScFps_ = new QComboBox(ctrlCard);
    cmbScFps_->addItem(QStringLiteral("Source Hz"), 0);
    cmbScFps_->addItem(QStringLiteral("60 FPS"), 60);
    cmbScFps_->addItem(QStringLiteral("30 FPS"), 30);
    cmbScFps_->addItem(QStringLiteral("15 FPS"), 15);
    if (int idx = cmbScFps_->findData(savedFps); idx >= 0) cmbScFps_->setCurrentIndex(idx);
    connect(cmbScFps_, &QComboBox::currentIndexChanged, this, [this]() {
        saveScreenCastPrefs();
        const int fps = cmbScFps_->currentData().toInt();
        scTimer_->setInterval(fps > 0 ? std::max(16, static_cast<int>(std::lround(1000.0 / fps))) : 16);
        if (chkScCapBroker_ && chkScCapBroker_->isChecked()) sendPwCastLimitsCmd();
        if (btnScLive_ && btnScLive_->isChecked()) sendPwPreviewCmd(true);
        updateScProfileBadge();
    });
    r1->addWidget(cmbScFps_);
    ctrlCol->addLayout(r1);

    auto* r2 = new QHBoxLayout();
    chkScCapBroker_ = new QCheckBox(QStringLiteral("Cap Real Discord/OBS Cast"), ctrlCard);
    chkScCapBroker_->setChecked(savedCap);
    connect(chkScCapBroker_, &QCheckBox::toggled, this, [this]() {
        saveScreenCastPrefs();
        sendPwCastLimitsCmd();
        updateScProfileBadge();
    });
    r2->addWidget(chkScCapBroker_);

    lblScProfileBadge_ = new QLabel(QStringLiteral("Profile: Source @ Source Hz"), ctrlCard);
    lblScProfileBadge_->setObjectName(QStringLiteral("DbgStat"));
    lblScProfileBadge_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    lblScProfileBadge_->setMinimumWidth(0);
    r2->addWidget(lblScProfileBadge_, 1);

    btnScLive_ = new QPushButton(QStringLiteral("Enable Live Feed (OFF)"), ctrlCard);
    btnScLive_->setCheckable(true);
    btnScLive_->setObjectName(QStringLiteral("PrimaryBtn"));
    connect(btnScLive_, &QPushButton::toggled, this, [this](bool on) {
        if (on) {
            btnScLive_->setText(QStringLiteral("Disable Live Feed (ON)"));
            lastShmSeq_ = 0;
            scFrameTimesMs_.clear();
            const int fps = cmbScFps_->currentData().toInt();
            scTimer_->setInterval(fps > 0 ? std::max(16, static_cast<int>(std::lround(1000.0 / fps))) : 16);
            sendPwPreviewCmd(true);
            scTimer_->start();
        } else {
            btnScLive_->setText(QStringLiteral("Enable Live Feed (OFF)"));
            scTimer_->stop();
            salsa_dbg_shm_stop(shmCtx_);
            sendPwPreviewCmd(false);
        }
    });
    r2->addWidget(btnScLive_);

    btnScSaveFrame_ = new QPushButton(QStringLiteral("Save Frame PNG"), ctrlCard);
    connect(btnScSaveFrame_, &QPushButton::clicked, this, [this]() {
        if (lastScImage_.isNull() || scRgbaBuf_.empty() || lastScW_ == 0 || lastScH_ == 0) {
            lblScStats_->setText(QStringLiteral("No live frame captured yet to save."));
            return;
        }
        const SalsaStr saved(salsa_dbg_save_frame_png(scRgbaBuf_.data(), lastScW_, lastScH_));
        lblScStats_->setText(saved.toQString());
    });
    r2->addWidget(btnScSaveFrame_);

    auto* btnPwTest = new QPushButton(QStringLiteral("Run PW Self-Test"), ctrlCard);
    connect(btnPwTest, &QPushButton::clicked, this, [this]() {
        lblScStats_->setText(SalsaStr(salsa_dbg_pw_self_test()).toQString());
    });
    r2->addWidget(btnPwTest);
    ctrlCol->addLayout(r2);

    auto* r3 = new QHBoxLayout();
    lblScStats_ = new QLabel(QStringLiteral("PW Video Broker: Waiting for snapshot..."), ctrlCard);
    lblScStats_->setObjectName(QStringLiteral("DbgStat"));
    lblScStats_->setSizePolicy(QSizePolicy::Ignored, QSizePolicy::Preferred);
    lblScStats_->setMinimumWidth(0);
    r3->addWidget(lblScStats_, 1);

    lblScDmabufBadge_ = new QLabel(QStringLiteral("IDLE"), ctrlCard);
    lblScDmabufBadge_->setObjectName(QStringLiteral("SavedChipLabel"));
    r3->addWidget(lblScDmabufBadge_);
    ctrlCol->addLayout(r3);
    lay->addWidget(ctrlCard);

    auto* viewCard = new QFrame(w);
    viewCard->setObjectName(QStringLiteral("DbgCard"));
    auto* vLay = new QVBoxLayout(viewCard);
    vLay->setContentsMargins(8, 8, 8, 8);
    lblScPreview_ = new QLabel(
        QStringLiteral("Live ScreenCast Preview is OFF (Zero Overhead).\nToggle 'Enable Live Feed' above to stream portal / dry-run frames."),
        viewCard);
    lblScPreview_->setAlignment(Qt::AlignCenter);
    lblScPreview_->setMinimumHeight(260);
    lblScPreview_->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Expanding);
    lblScPreview_->setStyleSheet(QStringLiteral("background-color: #090b0f; border-radius: 8px;"));
    vLay->addWidget(lblScPreview_, 1);
    lay->addWidget(viewCard, 1);

    updateScProfileBadge();
    return w;
}

void SalsaLiveDebuggerWidget::onClearEvents() {
    if (!allEvents_.empty()) {
        uint64_t maxId = clearWatermarkId_;
        for (const auto& ev : allEvents_) maxId = std::max(maxId, ev.id);
        clearWatermarkId_ = maxId;
    }
    clearExtAfterMs_ = QDateTime::currentMSecsSinceEpoch();
    allEvents_.clear();
    displayedEvents_.clear();
    tblEvents_->setRowCount(0);
    txtEventDetail_->clear();
}

void SalsaLiveDebuggerWidget::onPollTick() {
    if (!dbg_) return;
    latestStats_ = parseJsonObj(SalsaStr(salsa_dbg_stats_json(dbg_)).toQString());
    latestSnapshot_ = parseJsonObj(SalsaStr(salsa_dbg_latest_snapshot_json(dbg_)).toQString());

    connOk_ = latestStats_.value(QStringLiteral("connected")).toBool(false);
    const QString sock = latestStats_.value(QStringLiteral("sock_path")).toString();
    const uint64_t rxTotal = static_cast<uint64_t>(latestStats_.value(QStringLiteral("total_events_rx")).toInteger(0));
    const uint64_t dropped = static_cast<uint64_t>(latestStats_.value(QStringLiteral("dropped_events")).toInteger(0));
    connSummary_ = connOk_
                       ? QStringLiteral("CONNECTED (%1) | rx=%2 dropped=%3").arg(sock).arg(rxTotal).arg(dropped)
                       : QStringLiteral("DISCONNECTED (%1): %2").arg(sock, latestStats_.value(QStringLiteral("last_error")).toString());
    lblConn_->setText(connOk_ ? QStringLiteral("LIVE: %1").arg(sock) : connSummary_);
    lblConn_->setStyleSheet(QStringLiteral("color: %1;").arg(
        theme_.color(connOk_ ? "accent_primary" : "accent_hot", connOk_ ? "#00e676" : "#fb7185")));

    const QJsonObject rt = latestSnapshot_.value(QStringLiteral("rt")).toObject();
    if (!rt.isEmpty()) {
        lblRtPill_->setText(
            QStringLiteral("RT: last %1µs | avg %2µs | max %3µs | budget %4µs | load %5% | rx %6")
                .arg(rt.value(QStringLiteral("last_cycle_us")).toDouble(), 0, 'f', 1)
                .arg(rt.value(QStringLiteral("avg_cycle_us")).toDouble(), 0, 'f', 1)
                .arg(rt.value(QStringLiteral("max_cycle_us")).toDouble(), 0, 'f', 1)
                .arg(rt.value(QStringLiteral("budget_us")).toDouble(), 0, 'f', 1)
                .arg(rt.value(QStringLiteral("cpu_load_pct")).toDouble(), 0, 'f', 2)
                .arg(rxTotal));
    } else {
        lblRtPill_->setText(connSummary_);
    }
    emit statusUpdated(connOk_, connSummary_);

    if (!paused_) {
        const QJsonArray evArr = parseJsonArr(SalsaStr(salsa_dbg_recent_events_json(dbg_, 4096)).toQString());
        allEvents_.clear();
        allEvents_.reserve(static_cast<size_t>(evArr.size()) + 128);
        for (const QJsonValue& v : evArr) {
            const QJsonObject o = v.toObject();
            const uint64_t id = static_cast<uint64_t>(o.value(QStringLiteral("id")).toInteger(0));
            if (id <= clearWatermarkId_) continue;
            DbgEventRow row;
            row.id = id;
            row.tsMs = o.value(QStringLiteral("ts_ms")).toInteger(0);
            row.sub = o.value(QStringLiteral("sub")).toString();
            row.lvl = o.value(QStringLiteral("lvl")).toString();
            row.pid = static_cast<uint32_t>(o.value(QStringLiteral("pid")).toInteger(0));
            row.sid = static_cast<uint32_t>(o.value(QStringLiteral("sid")).toInteger(0));
            row.v1 = static_cast<int32_t>(o.value(QStringLiteral("v1")).toInteger(0));
            row.v2 = static_cast<int32_t>(o.value(QStringLiteral("v2")).toInteger(0));
            row.hr = static_cast<int32_t>(o.value(QStringLiteral("hr")).toInteger(0));
            row.op = o.value(QStringLiteral("op")).toString();
            row.target = o.value(QStringLiteral("target")).toString();
            row.detail = o.value(QStringLiteral("detail")).toString();
            row.jsonPretty = QString::fromUtf8(QJsonDocument(o).toJson(QJsonDocument::Indented));
            allEvents_.push_back(std::move(row));
        }

        const bool wantJ = chkJournal_ && chkJournal_->isChecked();
        const bool wantA = chkAutosync_ && chkAutosync_->isChecked();
        const bool wantD = chkDbus_ && chkDbus_->isChecked();
        if ((wantJ || wantA || wantD) && subtabs_->currentIndex() == 0) {
            const QJsonArray extArr = parseJsonArr(SalsaStr(salsa_dbg_ext_logs_json(wantJ, wantA, wantD)).toQString());
            for (const QJsonValue& v : extArr) {
                const QJsonObject o = v.toObject();
                const qint64 ts = o.value(QStringLiteral("ts_ms")).toInteger(0);
                if (clearExtAfterMs_ > 0 && ts == 0) continue;  // hide static tail history immediately after Clear until toggled
                DbgEventRow row;
                row.tsMs = ts;
                row.sub = o.value(QStringLiteral("sub")).toString();
                row.lvl = o.value(QStringLiteral("lvl")).toString();
                row.op = o.value(QStringLiteral("op")).toString();
                row.target = o.value(QStringLiteral("target")).toString();
                row.detail = o.value(QStringLiteral("detail")).toString();
                row.jsonPretty = QString::fromUtf8(QJsonDocument(o).toJson(QJsonDocument::Indented));
                allEvents_.push_back(std::move(row));
            }
        }
        if (subtabs_->currentIndex() == 0) rebuildEventTable();
    }

    if (subtabs_->currentIndex() == 2) renderTelemetryTab();
    if (subtabs_->currentIndex() == 4) {
        const QJsonObject pw = latestSnapshot_.value(QStringLiteral("pw")).toObject();
        const QJsonValue brkV = pw.value(QStringLiteral("broker_running"));
        const bool brkOn = brkV.toBool(false) || (brkV.toInteger(0) != 0);
        const int dmabufN = pw.value(QStringLiteral("dmabuf_streams")).toInt(0);
        const int memfdN = pw.value(QStringLiteral("memfd_streams")).toInt(0);
        const int rejN = pw.value(QStringLiteral("audio_rejects")).toInt(0);
        const double actFps = pw.value(QStringLiteral("actual_fps_x100")).toDouble(0.0) / 100.0;
        if (!btnScLive_->isChecked()) {
            lblScStats_->setText(
                QStringLiteral("PW Broker: %1 | Clients: %2 | Streams: %3 DMA / %4 MemFd | Relayed: %5 (%6 FPS) | Audio Rejects: %7")
                    .arg(brkOn ? QStringLiteral("ONLINE") : QStringLiteral("OFFLINE"))
                    .arg(pw.value(QStringLiteral("clients")).toArray().size())
                    .arg(dmabufN).arg(memfdN)
                    .arg(pw.value(QStringLiteral("frames_relayed_total")).toInteger(0))
                    .arg(actFps, 0, 'f', 1)
                    .arg(rejN));
        }
    }
}

void SalsaLiveDebuggerWidget::rebuildEventTable() {
    const QString wantSub = cmbSub_->currentText();
    const QString minLvl = cmbLvl_->currentText();
    const int minRank = salsa_dbg_level_rank(minLvl.toUtf8().constData());
    const QString q = txtSearch_->text().trimmed().toLower();

    displayedEvents_.clear();
    for (const auto& ev : allEvents_) {
        if (wantSub != QLatin1String("ALL") && ev.sub != wantSub) continue;
        if (salsa_dbg_level_rank(ev.lvl.toUtf8().constData()) < minRank) continue;
        if (!q.isEmpty()) {
            const QString hay = QStringLiteral("%1 %2 %3 %4 %5")
                                    .arg(ev.pid)
                                    .arg(ev.op, ev.target, ev.detail, ev.sub)
                                    .toLower();
            if (!hay.contains(q)) continue;
        }
        displayedEvents_.push_back(ev);
    }

    const size_t cap = std::min<size_t>(displayedEvents_.size(), 500);
    const size_t start = displayedEvents_.size() > cap ? displayedEvents_.size() - cap : 0;
    if (start > 0) {
        displayedEvents_.erase(displayedEvents_.begin(), displayedEvents_.begin() + static_cast<ptrdiff_t>(start));
    }

    const bool atBottom = tblEvents_->verticalScrollBar()->value() >= tblEvents_->verticalScrollBar()->maximum() - 4;
    tblEvents_->setUpdatesEnabled(false);
    tblEvents_->setRowCount(static_cast<int>(displayedEvents_.size()));
    for (int r = 0; r < static_cast<int>(displayedEvents_.size()); ++r) {
        const auto& ev = displayedEvents_[static_cast<size_t>(r)];
        const QString tsStr = ev.tsMs > 0 ? SalsaStr(salsa_dbg_format_ts_ms(ev.tsMs)).toQString() : QStringLiteral("--:--:--");
        const QString vals = QStringLiteral("v=%1,%2 hr=0x%3").arg(ev.v1).arg(ev.v2).arg(QString::number(static_cast<uint32_t>(ev.hr), 16));
        tblEvents_->setItem(r, 0, makeRoItem(tsStr));
        tblEvents_->setItem(r, 1, makeRoItem(ev.sub));
        auto* lvlIt = makeRoItem(ev.lvl);
        if (ev.lvl == QLatin1String("ERROR")) lvlIt->setForeground(theme_.qcolor("accent_hot", "#fb7185"));
        else if (ev.lvl == QLatin1String("WARN")) lvlIt->setForeground(theme_.qcolor("accent_warn", "#f59e0b"));
        else if (ev.lvl == QLatin1String("INFO")) lvlIt->setForeground(theme_.qcolor("accent_primary", "#00e676"));
        tblEvents_->setItem(r, 2, lvlIt);
        tblEvents_->setItem(r, 3, makeRoItem(ev.op));
        tblEvents_->setItem(r, 4, makeRoItem(ev.target));
        tblEvents_->setItem(r, 5, makeRoItem(ev.pid ? QString::number(ev.pid) : QString()));
        tblEvents_->setItem(r, 6, makeRoItem(ev.sid ? QString::number(ev.sid) : QString()));
        tblEvents_->setItem(r, 7, makeRoItem(vals));
        tblEvents_->setItem(r, 8, makeRoItem(ev.detail));
    }
    tblEvents_->setUpdatesEnabled(true);
    if (chkAutoscroll_->isChecked() || atBottom) tblEvents_->scrollToBottom();
}

void SalsaLiveDebuggerWidget::renderProcTable() {
    const QString flt = cmbProcFilter_->currentText();
    const QString q = txtProcSearch_->text().trimmed().toLower();
    const bool wasSorted = tblProcs_->isSortingEnabled();
    tblProcs_->setSortingEnabled(false);
    tblProcs_->setUpdatesEnabled(false);
    tblProcs_->setRowCount(0);

    for (const QJsonValue& pv : latestProcs_) {
        const QJsonObject obj = pv.toObject();
        const qint64 pid = obj.value(QStringLiteral("pid")).toInteger(0);
        const QString comm = obj.value(QStringLiteral("comm")).toString();
        const QString cls = obj.value(QStringLiteral("classification")).toString();
        const QString shm = obj.value(QStringLiteral("sockets_shm")).toString();
        const QString alsa = obj.value(QStringLiteral("alsa_fds")).toString();
        const QString libs = obj.value(QStringLiteral("mapped_libs")).toString();
        const QString detail = obj.value(QStringLiteral("detail")).toString();

        if (flt.startsWith(QLatin1String("Leaks")) && !cls.contains(QStringLiteral("LEAK"), Qt::CaseInsensitive) && alsa.isEmpty()) {
            continue;
        }
        if (flt.startsWith(QLatin1String("Salsa")) && !cls.contains(QStringLiteral("SALSA"), Qt::CaseInsensitive) && !cls.contains(QStringLiteral("SHIM"), Qt::CaseInsensitive)) {
            continue;
        }
        if (!q.isEmpty()) {
            const QString hay = QStringLiteral("%1 %2 %3 %4 %5").arg(pid).arg(comm, cls, libs, detail).toLower();
            if (!hay.contains(q)) continue;
        }

        const int r = tblProcs_->rowCount();
        tblProcs_->insertRow(r);
        tblProcs_->setItem(r, 0, new NumericTableItem(pid));
        tblProcs_->setItem(r, 1, makeRoItem(comm));
        auto* clsIt = makeRoItem(cls);
        if (cls.contains(QStringLiteral("LEAK"), Qt::CaseInsensitive)) clsIt->setForeground(theme_.qcolor("accent_hot", "#fb7185"));
        else if (cls.contains(QStringLiteral("SALSA"), Qt::CaseInsensitive)) clsIt->setForeground(theme_.qcolor("accent_primary", "#00e676"));
        tblProcs_->setItem(r, 2, clsIt);
        tblProcs_->setItem(r, 3, makeRoItem(shm));
        tblProcs_->setItem(r, 4, makeRoItem(alsa));
        tblProcs_->setItem(r, 5, makeRoItem(libs.isEmpty() ? detail : libs));
    }
    tblProcs_->setUpdatesEnabled(true);
    tblProcs_->setSortingEnabled(wasSorted);
}

void SalsaLiveDebuggerWidget::renderTelemetryTab() {
    const QJsonObject eng = latestSnapshot_.value(QStringLiteral("engine")).toObject();
    const QJsonObject rt = latestSnapshot_.value(QStringLiteral("rt")).toObject();
    const QJsonObject bt = latestSnapshot_.value(QStringLiteral("bluez")).toObject();
    const QJsonObject cap = latestSnapshot_.value(QStringLiteral("capture")).toObject();
    const QJsonObject midi = latestSnapshot_.value(QStringLiteral("midi")).toObject();

    const double loadPct = rt.value(QStringLiteral("cpu_load_pct")).toDouble(0.0);
    barRtLoad_->setValue(std::clamp(static_cast<int>(std::lround(loadPct * 10.0)), 0, 1000));
    barRtLoad_->setFormat(QStringLiteral("%1% RT CPU").arg(loadPct, 0, 'f', 2));
    lblEngSummary_->setText(
        QStringLiteral("Sink: %1\nClock: %2 Hz / %3-bit | Quantum: %4f (%5 ch)\nCycles: %6 | XRUNs: %7 | PA v35: %8 (%9 clients)\nRT eq=%10µs mix=%11µs hw=%12µs")
            .arg(eng.value(QStringLiteral("hw_device")).toString())
            .arg(eng.value(QStringLiteral("sample_rate")).toInteger(48000))
            .arg(eng.value(QStringLiteral("format_bits")).toInteger(32))
            .arg(eng.value(QStringLiteral("period_frames")).toInteger(256))
            .arg(eng.value(QStringLiteral("channels")).toInteger(2))
            .arg(eng.value(QStringLiteral("total_cycles")).toInteger(0))
            .arg(eng.value(QStringLiteral("hw_xruns")).toInteger(0))
            .arg(eng.value(QStringLiteral("pa_server_running")).toInteger(0) ? QStringLiteral("ON") : QStringLiteral("OFF"))
            .arg(eng.value(QStringLiteral("pa_clients")).toInteger(0))
            .arg(rt.value(QStringLiteral("eq_us")).toDouble(0.0), 0, 'f', 1)
            .arg(rt.value(QStringLiteral("pa_mix_us")).toDouble(0.0), 0, 'f', 1)
            .arg(rt.value(QStringLiteral("hw_write_us")).toDouble(0.0), 0, 'f', 1));

    const bool btOn = bt.value(QStringLiteral("active")).toInteger(0) != 0;
    lblBtSummary_->setText(
        btOn ? QStringLiteral("Device: %1 (%2)\nRate: %3 Hz | Delay: %4 ms | MTU W: %5\nRing Fill: %6 / %7 | Packets: %8\nUnderruns: %9 | TX Errors: %10")
                   .arg(bt.value(QStringLiteral("alias")).toString(),
                        bt.value(QStringLiteral("codec")).toString())
                   .arg(bt.value(QStringLiteral("sample_rate")).toInteger(48000))
                   .arg(bt.value(QStringLiteral("delay_ms")).toInteger(0))
                   .arg(bt.value(QStringLiteral("mtu_w")).toInteger(0))
                   .arg(bt.value(QStringLiteral("tx_ring_fill")).toInteger(0))
                   .arg(bt.value(QStringLiteral("tx_ring_cap")).toInteger(0))
                   .arg(bt.value(QStringLiteral("tx_packets")).toInteger(0))
                   .arg(bt.value(QStringLiteral("tx_underruns")).toInteger(0))
                   .arg(bt.value(QStringLiteral("tx_errors")).toInteger(0))
             : QStringLiteral("Bluetooth A2DP Standby (Wired endpoint active)"));

    lblCapMidiSummary_->setText(
        QStringLiteral("Mic: %1 (%2 Hz hw / %3 Hz measured)\nCap Ring: %4 frames | Kicks: %5 | Duplex Lock: %6\nMIDI: %7 (%8) | Port %9:%10\nMIDI Routed: %11 events")
            .arg(cap.value(QStringLiteral("alias")).toString())
            .arg(cap.value(QStringLiteral("hw_rate")).toInteger(0))
            .arg(cap.value(QStringLiteral("measured_rate")).toInteger(0))
            .arg(cap.value(QStringLiteral("ring_fill")).toInteger(0))
            .arg(cap.value(QStringLiteral("kick_count")).toInteger(0))
            .arg(cap.value(QStringLiteral("duplex_locked")).toInteger(0) ? QStringLiteral("YES") : QStringLiteral("NO"))
            .arg(midi.value(QStringLiteral("controller")).toString())
            .arg(midi.value(QStringLiteral("dev_path")).toString())
            .arg(midi.value(QStringLiteral("seq_client")).toInteger(0))
            .arg(midi.value(QStringLiteral("seq_in_port")).toInteger(0))
            .arg(midi.value(QStringLiteral("events_routed")).toInteger(0)));

    const QJsonArray wsArr = latestSnapshot_.value(QStringLiteral("wasapi_sessions")).toArray();
    tblWasapi_->setRowCount(wsArr.size());
    for (int i = 0; i < wsArr.size(); ++i) {
        const QJsonObject s = wsArr[i].toObject();
        tblWasapi_->setItem(i, 0, makeRoItem(QString::number(s.value(QStringLiteral("session_id")).toInteger())));
        tblWasapi_->setItem(i, 1, makeRoItem(QString::number(s.value(QStringLiteral("pid")).toInteger())));
        tblWasapi_->setItem(i, 2, makeRoItem(s.value(QStringLiteral("name")).toString()));
        tblWasapi_->setItem(i, 3, makeRoItem(s.value(QStringLiteral("is_capture")).toInteger() ? QStringLiteral("CAP") : QStringLiteral("PB")));
        tblWasapi_->setItem(i, 4, makeRoItem(s.value(QStringLiteral("share_mode")).toInteger() == 1 ? QStringLiteral("EXCL") : QStringLiteral("SHRD")));
        tblWasapi_->setItem(i, 5, makeRoItem(QStringLiteral("%1 Hz / %2ch").arg(s.value(QStringLiteral("rate")).toInteger()).arg(s.value(QStringLiteral("channels")).toInteger())));
        tblWasapi_->setItem(i, 6, makeRoItem(QStringLiteral("%1 / %2f").arg(s.value(QStringLiteral("buffered_frames")).toInteger()).arg(s.value(QStringLiteral("buffer_frames")).toInteger())));
        tblWasapi_->setItem(i, 7, makeRoItem(QStringLiteral("xr=%1 vol=%2%").arg(s.value(QStringLiteral("xruns")).toInteger()).arg(s.value(QStringLiteral("volume_milli")).toInteger() / 10)));
    }

    const QJsonArray pcArr = latestSnapshot_.value(QStringLiteral("pa_clients")).toArray();
    tblPaClients_->setRowCount(pcArr.size());
    for (int i = 0; i < pcArr.size(); ++i) {
        const QJsonObject c = pcArr[i].toObject();
        tblPaClients_->setItem(i, 0, makeRoItem(QString::number(c.value(QStringLiteral("index")).toInteger())));
        tblPaClients_->setItem(i, 1, makeRoItem(QString::number(c.value(QStringLiteral("pid")).toInteger())));
        tblPaClients_->setItem(i, 2, makeRoItem(c.value(QStringLiteral("name")).toString()));
        tblPaClients_->setItem(i, 3, makeRoItem(QStringLiteral("v%1").arg(c.value(QStringLiteral("version")).toInteger())));
        tblPaClients_->setItem(i, 4, makeRoItem(QString::number(c.value(QStringLiteral("out_count")).toInteger())));
    }

    const QJsonArray psArr = latestSnapshot_.value(QStringLiteral("pa_streams")).toArray();
    tblPaStreams_->setRowCount(psArr.size());
    for (int i = 0; i < psArr.size(); ++i) {
        const QJsonObject st = psArr[i].toObject();
        tblPaStreams_->setItem(i, 0, makeRoItem(QString::number(st.value(QStringLiteral("index")).toInteger())));
        tblPaStreams_->setItem(i, 1, makeRoItem(QString::number(st.value(QStringLiteral("pid")).toInteger())));
        tblPaStreams_->setItem(i, 2, makeRoItem(st.value(QStringLiteral("name")).toString()));
        tblPaStreams_->setItem(i, 3, makeRoItem(st.value(QStringLiteral("is_playback")).toInteger() ? QStringLiteral("PB") : QStringLiteral("CAP")));
        tblPaStreams_->setItem(i, 4, makeRoItem(QStringLiteral("%1/%2ch").arg(st.value(QStringLiteral("rate")).toInteger()).arg(st.value(QStringLiteral("channels")).toInteger())));
        tblPaStreams_->setItem(i, 5, makeRoItem(QStringLiteral("%1 / %2").arg(st.value(QStringLiteral("buffered_frames")).toInteger()).arg(st.value(QStringLiteral("ring_cap_frames")).toInteger())));
        tblPaStreams_->setItem(i, 6, makeRoItem(QStringLiteral("%1% L:%2").arg(st.value(QStringLiteral("volume_milli")).toInteger() / 10).arg(st.value(QStringLiteral("mix_peak_l")).toDouble(), 0, 'f', 2)));
    }
}

void SalsaLiveDebuggerWidget::refreshBoundaryAudit() {
    latestAudit_ = parseJsonObj(SalsaStr(salsa_dbg_full_audit_json()).toQString());
    const QJsonObject audit = latestAudit_.value(QStringLiteral("audit")).toObject();
    latestProcs_ = audit.value(QStringLiteral("processes")).toArray();
    renderProcTable();

    const QJsonObject fresh = audit.value(QStringLiteral("build_freshness")).toObject();
    const bool stale = fresh.value(QStringLiteral("is_stale")).toBool(false);
    lblStaleWarn_->setText(
        stale ? QStringLiteral("WARNING — Build artifacts look older than source edits: %1").arg(fresh.value(QStringLiteral("summary")).toString())
              : QStringLiteral("Build & Binary Freshness: UP TO DATE (%1)").arg(audit.value(QStringLiteral("pipewire_note")).toString()));
    lblStaleWarn_->setStyleSheet(QStringLiteral("color: %1;").arg(
        theme_.color(stale ? "accent_warn" : "accent_primary", stale ? "#f59e0b" : "#00e676")));

    const QJsonArray units = audit.value(QStringLiteral("systemd_units")).toArray();
    tblUnits_->setRowCount(units.size());
    for (int i = 0; i < units.size(); ++i) {
        const QJsonObject u = units[i].toObject();
        tblUnits_->setItem(i, 0, makeRoItem(u.value(QStringLiteral("unit")).toString()));
        tblUnits_->setItem(i, 1, makeRoItem(u.value(QStringLiteral("active")).toString()));
        tblUnits_->setItem(i, 2, makeRoItem(u.value(QStringLiteral("sub_state")).toString()));
        tblUnits_->setItem(i, 3, makeRoItem(u.value(QStringLiteral("enabled")).toString()));
        tblUnits_->setItem(i, 4, makeRoItem(QString::number(u.value(QStringLiteral("main_pid")).toInteger(0))));
    }

    const QJsonArray socks = audit.value(QStringLiteral("sockets")).toArray();
    tblSockets_->setRowCount(socks.size());
    for (int i = 0; i < socks.size(); ++i) {
        const QJsonObject sk = socks[i].toObject();
        const bool ok = sk.value(QStringLiteral("connectable")).toBool(false);
        tblSockets_->setItem(i, 0, makeRoItem(sk.value(QStringLiteral("name")).toString()));
        tblSockets_->setItem(i, 1, makeRoItem(ok ? QStringLiteral("ONLINE") : QStringLiteral("DOWN")));
        tblSockets_->setItem(i, 2, makeRoItem(sk.value(QStringLiteral("peer_pid")).isNull() ? QString() : QString::number(sk.value(QStringLiteral("peer_pid")).toInteger())));
        tblSockets_->setItem(i, 3, makeRoItem(sk.value(QStringLiteral("peer_comm")).toString()));
        tblSockets_->setItem(i, 4, makeRoItem(sk.value(QStringLiteral("path")).toString()));
    }
    renderConfigViewer();
}

void SalsaLiveDebuggerWidget::renderConfigViewer() {
    if (!cmbCfg_ || !txtCfgView_) return;
    const QString key = cmbCfg_->currentText();
    const QJsonObject texts = latestAudit_.value(QStringLiteral("config_texts")).toObject();
    if (key.startsWith(QLatin1String("ALSA"))) {
        const QJsonObject audit = latestAudit_.value(QStringLiteral("audit")).toObject();
        txtCfgView_->setPlainText(
            QString::fromUtf8(QJsonDocument(audit.value(QStringLiteral("alsa")).toObject()).toJson(QJsonDocument::Indented))
            + QStringLiteral("\n\n")
            + QString::fromUtf8(QJsonDocument(audit.value(QStringLiteral("build_freshness")).toObject()).toJson(QJsonDocument::Indented)));
        return;
    }
    txtCfgView_->setPlainText(texts.value(key).toString(QStringLiteral("(Not present on disk or empty)")));
}

void SalsaLiveDebuggerWidget::sendPwPreviewCmd(bool enabled) {
    if (!dbg_) return;
    if (!enabled) {
        salsa_dbg_send_command(dbg_, "PW_PREVIEW 0");
        return;
    }
    const QJsonObject rObj = parseJsonObj(cmbScRes_->currentData().toString());
    const uint32_t mw = static_cast<uint32_t>(rObj.value(QStringLiteral("w")).toInteger(0));
    const uint32_t mh = static_cast<uint32_t>(rObj.value(QStringLiteral("h")).toInteger(0));
    const uint32_t fps = static_cast<uint32_t>(qvarInt(cmbScFps_->currentData(), 0));
    const QByteArray cmd = QStringLiteral("PW_PREVIEW 1 15000 %1 %2 %3").arg(mw).arg(mh).arg(fps).toUtf8();
    salsa_dbg_send_command(dbg_, cmd.constData());
}

void SalsaLiveDebuggerWidget::sendPwCastLimitsCmd() {
    if (!dbg_) return;
    scLimitsDirty_ = true;
    if (chkScCapBroker_ && chkScCapBroker_->isChecked()) {
        const QJsonObject rObj = parseJsonObj(cmbScRes_->currentData().toString());
        const uint32_t mw = static_cast<uint32_t>(rObj.value(QStringLiteral("w")).toInteger(0));
        const uint32_t mh = static_cast<uint32_t>(rObj.value(QStringLiteral("h")).toInteger(0));
        const uint32_t fps = static_cast<uint32_t>(qvarInt(cmbScFps_->currentData(), 0));
        const QByteArray cmd = QStringLiteral("PW_CAST_LIMITS %1 %2 %3").arg(mw).arg(mh).arg(fps).toUtf8();
        salsa_dbg_send_command(dbg_, cmd.constData());
    } else {
        salsa_dbg_send_command(dbg_, "PW_CAST_LIMITS 0 0 0");
    }
}

void SalsaLiveDebuggerWidget::saveScreenCastPrefs() {
    const QJsonObject rObj = parseJsonObj(cmbScRes_->currentData().toString());
    const QString rk = rObj.value(QStringLiteral("key")).toString(QStringLiteral("Source"));
    const uint32_t fps = static_cast<uint32_t>(qvarInt(cmbScFps_->currentData(), 60));
    const bool capReal = chkScCapBroker_ && chkScCapBroker_->isChecked();
    salsa_dbg_save_sc_prefs(rk.toUtf8().constData(), fps, capReal);
}

void SalsaLiveDebuggerWidget::updateScProfileBadge(double measuredFps, const QString& activeDims) {
    if (!lblScProfileBadge_ || !cmbScRes_ || !cmbScFps_) return;
    const QJsonObject rObj = parseJsonObj(cmbScRes_->currentData().toString());
    const QString rk = rObj.value(QStringLiteral("key")).toString(QStringLiteral("Source"));
    const int fps = qvarInt(cmbScFps_->currentData(), 0);
    const bool cap = chkScCapBroker_ && chkScCapBroker_->isChecked();
    const QString fpsStr = fps <= 0 ? QStringLiteral("Source Hz") : QStringLiteral("%1 FPS").arg(fps);
    const QString dimPart = activeDims.isEmpty() ? QString() : QStringLiteral(" (%1)").arg(activeDims);
    const QString livePart = measuredFps > 0.1 ? QStringLiteral(" | Live: %1 FPS").arg(measuredFps, 0, 'f', 1) : QString();
    const QString capPart = cap ? QStringLiteral(" | Broker Cap: ON") : QString();
    lblScProfileBadge_->setText(QStringLiteral("Profile: %1%2 @ %3%4%5").arg(rk, dimPart, fpsStr, livePart, capPart));
}

void SalsaLiveDebuggerWidget::onScreenCastTick() {
    if (!btnScLive_ || !btnScLive_->isChecked() || !shmCtx_) return;

    const QJsonObject rObj = parseJsonObj(cmbScRes_->currentData().toString());
    const uint32_t mw = static_cast<uint32_t>(rObj.value(QStringLiteral("w")).toInteger(0));
    const uint32_t mh = static_cast<uint32_t>(rObj.value(QStringLiteral("h")).toInteger(0));
    const uint32_t fps = static_cast<uint32_t>(qvarInt(cmbScFps_->currentData(), 0));
    const uint32_t capFps = (chkScCapBroker_ && chkScCapBroker_->isChecked()) ? fps : 0;

    static constexpr size_t kMaxRgbaBytes = 3840ULL * 2160ULL * 4ULL;
    if (scRgbaBuf_.size() < kMaxRgbaBytes) scRgbaBuf_.resize(kMaxRgbaBytes);

    const QString mode = cmbScMode_->currentText();
    const bool allowShm = mode.startsWith(QLatin1String("Auto")) || mode.contains(QLatin1String("SHM"));
    const bool forceDry = mode.contains(QLatin1String("Dry-Run"));

    const QJsonObject pw = latestSnapshot_.value(QStringLiteral("pw")).toObject();
    const QByteArray shmName = pw.value(QStringLiteral("preview_shm_name")).toString().toUtf8();

    SalsaScFrameMeta meta{};
    bool gotFrame = false;
    bool usingDry = false;
    uint32_t fw = 0, fh = 0;

    if (allowShm && !forceDry && !shmName.isEmpty()) {
        const int rc = salsa_dbg_shm_poll_frame(
            shmCtx_, shmName.constData(), mw, mh, fps, capFps,
            scRgbaBuf_.data(), scRgbaBuf_.size(), &meta);
        if (rc > 0 && meta.active_stream && meta.width >= 2 && meta.height >= 2 && meta.bytes_written >= static_cast<size_t>(meta.width) * meta.height * 4) {
            gotFrame = true;
            fw = meta.width;
            fh = meta.height;
        }
    }

    if (!gotFrame && (forceDry || mode.startsWith(QLatin1String("Auto")))) {
        uint32_t sw = 1280, sh = 720;
        if (QScreen* scr = QGuiApplication::primaryScreen()) {
            const double dpr = scr->devicePixelRatio() > 0.5 ? scr->devicePixelRatio() : 1.0;
            sw = std::max<uint32_t>(320, static_cast<uint32_t>(std::lround(scr->size().width() * dpr)));
            sh = std::max<uint32_t>(240, static_cast<uint32_t>(std::lround(scr->size().height() * dpr)));
        }
        uint32_t dw = sw, dh = sh;
        const uint32_t cw = mw > 0 ? mw : 1920;
        const uint32_t ch = mh > 0 ? mh : 1080;
        if (dw > cw) { dh = std::max<uint32_t>(2, (dh * cw) / dw); dw = cw; }
        if (dh > ch) { dw = std::max<uint32_t>(2, (dw * ch) / dh); dh = ch; }
        dw = std::max<uint32_t>(2, dw & ~1U);
        dh = std::max<uint32_t>(2, dh & ~1U);
        const size_t n = salsa_dbg_dryrun_frame(shmCtx_, dw, dh, scRgbaBuf_.data(), scRgbaBuf_.size());
        if (n >= static_cast<size_t>(dw) * dh * 4) {
            gotFrame = true;
            usingDry = true;
            fw = dw;
            fh = dh;
        }
    }

    const qint64 nowMs = QDateTime::currentMSecsSinceEpoch();
    if (gotFrame) {
        scFrameTimesMs_.append(nowMs);
        while (scFrameTimesMs_.size() > 2 && (nowMs - scFrameTimesMs_.first()) > 1250) scFrameTimesMs_.removeFirst();
        double measFps = 0.0;
        if (scFrameTimesMs_.size() >= 2) {
            const double dt = (scFrameTimesMs_.last() - scFrameTimesMs_.first()) / 1000.0;
            if (dt > 0.05) measFps = (scFrameTimesMs_.size() - 1) / dt;
        }
        QImage rawImg(scRgbaBuf_.data(), static_cast<int>(fw), static_cast<int>(fh),
                      static_cast<int>(fw * 4), QImage::Format_RGBA8888);
        lastScImage_ = rawImg.copy();
        lastScW_ = fw;
        lastScH_ = fh;

        const QSize targetSz = lblScPreview_->size().expandedTo(QSize(320, 180));
        lblScPreview_->setPixmap(QPixmap::fromImage(
            lastScImage_.scaled(targetSz, Qt::KeepAspectRatio, Qt::SmoothTransformation)));

        updateScProfileBadge(measFps, QStringLiteral("%1x%2").arg(fw).arg(fh));
        if (usingDry) {
            lblScDmabufBadge_->setText(QStringLiteral("DRY-RUN SIM"));
            lblScStats_->setText(QStringLiteral("Dry-Run Portal Sim: %1x%2 @ %3 FPS (Zero-PipeWire Idle)")
                                     .arg(fw).arg(fh).arg(measFps, 0, 'f', 1));
        } else {
            const QString kindTag = meta.is_dmabuf
                                        ? (meta.dmabuf_mappable ? QStringLiteral("DMA-BUF Mappable") : QStringLiteral("DMA-BUF GPU-Opaque"))
                                        : QStringLiteral("MemFd SHM");
            lblScDmabufBadge_->setText(kindTag);
            lblScStats_->setText(
                QStringLiteral("Live Portal Cast: %1 (%2) | %3x%4 | Broker FPS=%5 | Seq=%6 | Cons=%7")
                    .arg(cFixedStr(meta.prod_name, sizeof(meta.prod_name)),
                         cFixedStr(meta.format_name, sizeof(meta.format_name)))
                    .arg(fw).arg(fh)
                    .arg(meta.actual_fps_x100 / 100.0, 0, 'f', 1)
                    .arg(meta.seq)
                    .arg(meta.cons_count));
        }
    } else if (allowShm && meta.active_stream && meta.is_dmabuf && !meta.dmabuf_mappable) {
        lblScDmabufBadge_->setText(QStringLiteral("DMA-BUF GPU-Opaque"));
        lblScStats_->setText(QStringLiteral("Portal stream active (%1x%2) uses GPU-opaque DMA-BUF (switch Mode to Dry-Run for software preview).").arg(meta.width).arg(meta.height));
    }
}

}  // namespace salsa

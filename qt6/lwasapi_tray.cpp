#include "salsa_common.hpp"

#include <QAbstractItemView>
#include <QAction>
#include <QActionGroup>
#include <QApplication>
#include <QButtonGroup>
#include <QComboBox>
#include <QDBusArgument>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusMetaType>
#include <QDBusObjectPath>
#include <QDBusReply>
#include <QDateTime>
#include <QDir>
#include <QEvent>
#include <QFile>
#include <QFileInfo>
#include <QFont>
#include <QFontMetricsF>
#include <QFrame>
#include <QGuiApplication>
#include <QHBoxLayout>
#include <QIcon>
#include <QKeyEvent>
#include <QLabel>
#include <QLayout>
#include <QLayoutItem>
#include <QLinearGradient>
#include <QMenu>
#include <QMetaObject>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPen>
#include <QPixmap>
#include <QPushButton>
#include <QRegularExpression>
#include <QScreen>
#include <QScrollArea>
#include <QSizePolicy>
#include <QStackedWidget>
#include <QTimer>
#include <QVBoxLayout>
#include <QWheelEvent>

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <thread>
#include <utility>
#include <vector>

using SalsaDbusIfaces = QMap<QString, QVariantMap>;
using SalsaDbusManagedObjects = QMap<QDBusObjectPath, SalsaDbusIfaces>;
Q_DECLARE_METATYPE(SalsaDbusIfaces)
Q_DECLARE_METATYPE(SalsaDbusManagedObjects)

namespace salsa {

inline qint64 nowMs() { return QDateTime::currentMSecsSinceEpoch(); }

inline constexpr int TRAY_SAMPLE_RATES[6] = {44100, 48000, 88200, 96000, 176400, 192000};
inline constexpr int TRAY_FONT_STEPS[7] = {80, 90, 100, 110, 120, 135, 150};

inline QPoint waylandToQtPoint(int gx, int gy) {
    if (QGuiApplication::platformName() != QLatin1String("xcb")) {
        return QPoint(gx, gy);
    }
    const auto screens = QGuiApplication::screens();
    for (QScreen* s : screens) {
        const QRect sg = s->geometry();
        const double dpr = std::max(1.0, static_cast<double>(s->devicePixelRatio()));
        const int wx = static_cast<int>(std::lround(sg.x() / dpr));
        const int wy = static_cast<int>(std::lround(sg.y() / dpr));
        if (wx <= gx && gx < wx + sg.width() && wy <= gy && gy < wy + sg.height()) {
            return QPoint(sg.x() + (gx - wx), sg.y() + (gy - wy));
        }
    }
    for (QScreen* s : screens) {
        const QRect sg = s->geometry();
        const double dpr = std::max(1.0, static_cast<double>(s->devicePixelRatio()));
        const double px = std::lround(sg.x() / dpr) * dpr;
        const double py = std::lround(sg.y() / dpr) * dpr;
        const double pw = sg.width() * dpr;
        const double ph = sg.height() * dpr;
        if (px <= gx && gx < px + pw && py <= gy && gy < py + ph) {
            return QPoint(
                sg.x() + static_cast<int>(std::lround((gx - px) / dpr)),
                sg.y() + static_cast<int>(std::lround((gy - py) / dpr)));
        }
    }
    return QPoint(gx, gy);
}

inline QScreen* screenForTrayPoint(const QPoint& rawPt, bool hasRawPt) {
    if (hasRawPt && (rawPt.x() != 0 || rawPt.y() != 0)) {
        const QPoint qtPt = waylandToQtPoint(rawPt.x(), rawPt.y());
        if (QScreen* s = QGuiApplication::screenAt(qtPt)) return s;
    }
    if (QScreen* s = QGuiApplication::screenAt(QCursor::pos())) return s;
    return QGuiApplication::primaryScreen();
}

static QFrame* makeHLine(QWidget* parent) {
    auto* f = new QFrame(parent);
    f->setObjectName(QStringLiteral("HLine"));
    f->setFrameShape(QFrame::NoFrame);
    f->setFixedHeight(1);
    return f;
}

static QWidget* makeSectionHeader(const QString& title, QWidget* parent) {
    auto* row = new QWidget(parent);
    auto* lay = new QHBoxLayout(row);
    lay->setContentsMargins(0, 2, 0, 0);
    lay->setSpacing(8);
    auto* lbl = new QLabel(title, row);
    lbl->setObjectName(QStringLiteral("SectionTitle"));
    lay->addWidget(lbl);
    lay->addWidget(makeHLine(row), 1);
    return row;
}

static QList<int> probeAlsaPlaybackRates(const QString& pbId) {
    static const QRegularExpression reHw(QStringLiteral("^hw:(\\d+),(\\d+)$"));
    const QRegularExpressionMatch m = reHw.match(pbId.trimmed());
    if (!m.hasMatch()) return {};

    const QString path = QStringLiteral("/proc/asound/card%1/stream0").arg(m.captured(1));
    QFile fp(path);
    if (!fp.open(QIODevice::ReadOnly | QIODevice::Text)) return {};

    bool inPb = false;
    QList<int> rates;
    static const QRegularExpression reNum(QStringLiteral("\\d+"));
    while (!fp.atEnd()) {
        const QString ln = QString::fromUtf8(fp.readLine()).trimmed();
        if (ln.startsWith(QLatin1String("Playback:"))) {
            inPb = true;
        } else if (ln.startsWith(QLatin1String("Capture:"))) {
            inPb = false;
        } else if (inPb && ln.startsWith(QLatin1String("Rates:"))) {
            if (ln.contains(QLatin1String("continuous"), Qt::CaseInsensitive) || ln.contains(QLatin1Char('-'))) {
                return {};
            }
            auto it = reNum.globalMatch(ln);
            while (it.hasNext()) {
                bool ok = false;
                const int v = it.next().captured(0).toInt(&ok);
                if (ok && v >= 8000 && v <= 768000 && !rates.contains(v)) {
                    rates.append(v);
                }
            }
        }
    }
    return rates;
}

struct BtBatteryInfo {
    int pct = -1;
    QString addr;
};

static int parseDbusPctVariant(const QVariant& raw) {
    if (!raw.isValid() || raw.isNull()) return -1;
    if (raw.typeId() == QMetaType::QByteArray) {
        const QByteArray ba = raw.toByteArray();
        return ba.isEmpty() ? -1 : static_cast<unsigned char>(ba.at(0));
    }
    bool ok = false;
    const double dv = raw.toDouble(&ok);
    if (ok && dv >= 0.0 && dv <= 100.0) {
        return static_cast<int>(std::lround(dv));
    }
    return -1;
}

static BtBatteryInfo readBluezBatteryPct(const QString& targetAlias) {
    static std::once_flag metaRegOnce;
    std::call_once(metaRegOnce, []() {
        qDBusRegisterMetaType<SalsaDbusIfaces>();
        qDBusRegisterMetaType<SalsaDbusManagedObjects>();
    });

    QDBusConnection bus = QDBusConnection::systemBus();
    if (!bus.isConnected()) return {};

    const QString targetL = targetAlias.trimmed().toLower();

    QDBusMessage msg = QDBusMessage::createMethodCall(
        QStringLiteral("org.bluez"),
        QStringLiteral("/"),
        QStringLiteral("org.freedesktop.DBus.ObjectManager"),
        QStringLiteral("GetManagedObjects"));
    QDBusReply<SalsaDbusManagedObjects> rep = bus.call(msg, QDBus::Block, 150);
    if (rep.isValid()) {
        struct Cand {
            int score;
            int pct;
            QString addr;
        };
        std::vector<Cand> candidates;
        const SalsaDbusManagedObjects objs = rep.value();
        for (auto it = objs.constBegin(); it != objs.constEnd(); ++it) {
            const SalsaDbusIfaces& ifaces = it.value();
            if (!ifaces.contains(QStringLiteral("org.bluez.Device1"))
                || !ifaces.contains(QStringLiteral("org.bluez.Battery1"))) {
                continue;
            }
            const QVariantMap dev = ifaces.value(QStringLiteral("org.bluez.Device1"));
            const QVariantMap bat = ifaces.value(QStringLiteral("org.bluez.Battery1"));
            if (!dev.value(QStringLiteral("Connected")).toBool()) continue;
            const int pct = parseDbusPctVariant(bat.value(QStringLiteral("Percentage")));
            if (pct < 0 || pct > 100) continue;

            const QString aliasL = dev.value(QStringLiteral("Alias")).toString().trimmed().toLower();
            const QString nameL = dev.value(QStringLiteral("Name")).toString().trimmed().toLower();
            const QString iconL = dev.value(QStringLiteral("Icon")).toString().trimmed().toLower();
            const QStringList uuids = dev.value(QStringLiteral("UUIDs")).toStringList();
            bool isAudio = (iconL == QLatin1String("audio-headset")
                            || iconL == QLatin1String("audio-headphones")
                            || iconL == QLatin1String("audio-card"));
            if (!isAudio) {
                for (const QString& u : uuids) {
                    const QString ul = u.toLower();
                    if (ul.startsWith(QLatin1String("0000110b-"))
                        || ul.startsWith(QLatin1String("0000110d-"))
                        || ul.startsWith(QLatin1String("0000111e-"))) {
                        isAudio = true;
                        break;
                    }
                }
            }
            int score = 0;
            if (!targetL.isEmpty()) {
                if (targetL == aliasL || targetL == nameL) score += 100;
                else if ((!aliasL.isEmpty() && (targetL.contains(aliasL) || aliasL.contains(targetL)))
                         || (!nameL.isEmpty() && (targetL.contains(nameL) || nameL.contains(targetL)))) {
                    score += 70;
                }
            }
            if (isAudio) score += 25;
            if (!targetL.isEmpty() && score == 0) continue;
            candidates.push_back({score, pct, dev.value(QStringLiteral("Address")).toString()});
        }
        if (!candidates.empty()) {
            std::sort(candidates.begin(), candidates.end(), [](const Cand& a, const Cand& b) {
                return a.score > b.score;
            });
            return {candidates.front().pct, candidates.front().addr};
        }
    }

    // UPower Bluetooth headset fallback
    QDBusMessage uMsg = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.UPower"),
        QStringLiteral("/org/freedesktop/UPower"),
        QStringLiteral("org.freedesktop.UPower"),
        QStringLiteral("EnumerateDevices"));
    QDBusReply<QList<QDBusObjectPath>> uRep = bus.call(uMsg, QDBus::Block, 150);
    if (uRep.isValid()) {
        for (const QDBusObjectPath& dpath : uRep.value()) {
            const QString pstr = dpath.path();
            const QString pl = pstr.toLower();
            if (!pl.contains(QLatin1String("bluez")) && !pl.contains(QLatin1String("headset"))) continue;
            QDBusMessage pCall = QDBusMessage::createMethodCall(
                QStringLiteral("org.freedesktop.UPower"),
                pstr,
                QStringLiteral("org.freedesktop.DBus.Properties"),
                QStringLiteral("GetAll"));
            pCall.setArguments({QStringLiteral("org.freedesktop.UPower.Device")});
            QDBusReply<QVariantMap> pRep = bus.call(pCall, QDBus::Block, 120);
            if (!pRep.isValid()) continue;
            const QVariantMap props = pRep.value();
            const QString modelL = props.value(QStringLiteral("Model")).toString().trimmed().toLower();
            if (!targetL.isEmpty() && !modelL.isEmpty()
                && !targetL.contains(modelL) && !modelL.contains(targetL)) {
                continue;
            }
            const int pct = parseDbusPctVariant(props.value(QStringLiteral("Percentage")));
            if (pct > 0 && pct <= 100) {
                return {pct, props.value(QStringLiteral("Serial")).toString()};
            }
        }
    }
    return {};
}

// =========================================================================
// AppIconResolver — /proc/<pid> + XDG .desktop + QIcon::fromTheme resolver
// =========================================================================

class AppIconResolver {
public:
    QPixmap pixmapFor(const QString& clientName, uint32_t clientPid, bool isCapture, int size) {
        const QString clean = clientName.trimmed();
        const QString key = QStringLiteral("%1|%2|%3|%4")
                                .arg(clientPid)
                                .arg(clean.toLower())
                                .arg(isCapture ? 1 : 0)
                                .arg(size);
        if (auto it = cache_.constFind(key); it != cache_.constEnd()) {
            return *it;
        }

        ensureDesktopIndex();

        QStringList candidates;
        auto addAlias = [&](const QString& tok) {
            if (tok.isEmpty()) return;
            candidates.append(tok);
            static const QMap<QString, QStringList> aliases = {
                {QStringLiteral("firefox"), {QStringLiteral("firefox"), QStringLiteral("firefox-esr"), QStringLiteral("firefox-developer-edition")}},
                {QStringLiteral("mozilla firefox"), {QStringLiteral("firefox")}},
                {QStringLiteral("zen"), {QStringLiteral("zen-browser"), QStringLiteral("zen")}},
                {QStringLiteral("discord"), {QStringLiteral("discord"), QStringLiteral("com.discordapp.Discord"), QStringLiteral("discord-canary"), QStringLiteral("discord-ptb")}},
                {QStringLiteral("vesktop"), {QStringLiteral("vesktop"), QStringLiteral("dev.vencord.Vesktop"), QStringLiteral("discord")}},
                {QStringLiteral("chrome"), {QStringLiteral("google-chrome"), QStringLiteral("chromium"), QStringLiteral("chromium-browser")}},
                {QStringLiteral("google chrome"), {QStringLiteral("google-chrome"), QStringLiteral("chromium")}},
                {QStringLiteral("chromium"), {QStringLiteral("chromium"), QStringLiteral("chromium-browser")}},
                {QStringLiteral("brave"), {QStringLiteral("brave-browser"), QStringLiteral("brave")}},
                {QStringLiteral("steam"), {QStringLiteral("steam"), QStringLiteral("com.valvesoftware.Steam")}},
                {QStringLiteral("spotify"), {QStringLiteral("spotify-client"), QStringLiteral("spotify"), QStringLiteral("com.spotify.Client")}},
                {QStringLiteral("obs"), {QStringLiteral("com.obsproject.Studio"), QStringLiteral("obs")}},
                {QStringLiteral("mpv"), {QStringLiteral("mpv")}},
                {QStringLiteral("vlc"), {QStringLiteral("vlc")}},
                {QStringLiteral("elisa"), {QStringLiteral("elisa")}},
                {QStringLiteral("haruna"), {QStringLiteral("haruna")}},
                {QStringLiteral("audacity"), {QStringLiteral("audacity")}},
                {QStringLiteral("reaper"), {QStringLiteral("cockos-reaper"), QStringLiteral("reaper")}},
                {QStringLiteral("lutris"), {QStringLiteral("lutris"), QStringLiteral("net.lutris.Lutris")}},
                {QStringLiteral("heroic"), {QStringLiteral("heroic"), QStringLiteral("com.heroicgameslauncher.hgl")}},
                {QStringLiteral("bottles"), {QStringLiteral("com.usebottles.bottles"), QStringLiteral("bottles")}},
            };
            if (auto ait = aliases.constFind(tok); ait != aliases.constEnd()) {
                candidates.append(*ait);
            }
        };

        if (clientPid > 0) {
            const QByteArray rawCmd = safeReadProc(clientPid, QStringLiteral("cmdline"), 4096);
            if (!rawCmd.isEmpty()) {
                const int nul = rawCmd.indexOf('\0');
                const QString argv0 = QString::fromUtf8(nul >= 0 ? rawCmd.left(nul) : rawCmd).trimmed();
                const QString base = QFileInfo(argv0).fileName().toLower();
                if (!base.isEmpty()
                    && base != QLatin1String("wine")
                    && base != QLatin1String("wine64")
                    && base != QLatin1String("python")
                    && base != QLatin1String("python3")
                    && base != QLatin1String("env")
                    && base != QLatin1String("sh")
                    && base != QLatin1String("bash")) {
                    addAlias(base);
                }
            }
            const QString comm = QString::fromUtf8(safeReadProc(clientPid, QStringLiteral("comm"), 256)).trimmed().toLower();
            if (!comm.isEmpty() && !comm.startsWith(QLatin1String("wine"))) {
                addAlias(comm);
            }
        }

        QString low = clean.toLower();
        if (low.startsWith(QLatin1String("[mic] "))) low = low.mid(6).trimmed();
        if (!low.isEmpty()) {
            addAlias(low);
            static const QRegularExpression reSep(QStringLiteral("[\\s\\-_/]+"));
            const QStringList toks = low.split(reSep, Qt::SkipEmptyParts);
            for (const QString& tok : toks) {
                if (tok.size() >= 3) addAlias(tok);
            }
        }

        QStringList ordered;
        for (const QString& c : candidates) {
            const QString trimmed = c.trimmed();
            if (!trimmed.isEmpty() && !ordered.contains(trimmed)) {
                ordered.append(trimmed);
            }
        }

        QPixmap pm;
        for (const QString& c : ordered) {
            for (const QMap<QString, QString>* m : {&byExec_, &byWm_, &byName_}) {
                if (auto it = m->constFind(c); it != m->constEnd()) {
                    pm = pixmapFromSpec(*it, size);
                    if (!pm.isNull()) break;
                }
            }
            if (!pm.isNull()) break;
            pm = pixmapFromSpec(c, size);
            if (!pm.isNull()) break;
        }

        if (pm.isNull()) {
            pm = pixmapFromSpec(
                isCapture ? QStringLiteral("audio-input-microphone") : QStringLiteral("applications-multimedia"),
                size);
        }

        if (pm.isNull()) {
            pm = QPixmap(size, size);
            pm.fill(Qt::transparent);
            QPainter p(&pm);
            p.setRenderHint(QPainter::Antialiasing, true);
            p.setBrush(QColor(QStringLiteral("#252c33")));
            p.setPen(QPen(QColor(isCapture ? QStringLiteral("#38bdf8") : QStringLiteral("#14b866")), 1.2));
            p.drawRoundedRect(QRectF(1.0, 1.0, size - 2.0, size - 2.0), 5.0, 5.0);
            p.setPen(QColor(QStringLiteral("#f8fafc")));
            QFont f = p.font();
            f.setBold(true);
            f.setPixelSize(std::max(9, static_cast<int>(size * 0.52)));
            p.setFont(f);
            const QString ch = !clean.isEmpty() ? clean.left(1).toUpper()
                                                : (isCapture ? QStringLiteral("M") : QStringLiteral("A"));
            p.drawText(pm.rect(), Qt::AlignCenter, ch);
        }

        if (cache_.size() > 256) cache_.clear();
        cache_.insert(key, pm);
        return pm;
    }

private:
    QByteArray safeReadProc(uint32_t pid, const QString& leaf, int maxBytes) const {
        if (pid == 0) return {};
        const QString p = QStringLiteral("/proc/%1/%2").arg(pid).arg(leaf);
        const QFileInfo fi(p);
        if (fi.isSymLink()) return {};
        QFile f(p);
        if (!f.open(QIODevice::ReadOnly)) return {};
        return f.read(maxBytes);
    }

    QPixmap pixmapFromSpec(const QString& spec, int size) const {
        if (spec.isEmpty()) return {};
        if (spec.contains(QLatin1Char('/'))) {
            const QFileInfo fi(spec);
            if (fi.isFile() && !fi.isSymLink()) {
                const QPixmap pm(spec);
                if (!pm.isNull()) {
                    return pm.scaled(size, size, Qt::KeepAspectRatio, Qt::SmoothTransformation);
                }
            }
            return {};
        }
        const QIcon ic = QIcon::fromTheme(spec);
        if (!ic.isNull()) {
            const QPixmap pm = ic.pixmap(size, size);
            if (!pm.isNull()) return pm;
        }
        return {};
    }

    void ensureDesktopIndex() {
        if (indexed_) return;
        indexed_ = true;
        const QString home = QDir::homePath();
        const QStringList dirs = {
            home + QStringLiteral("/.local/share/applications"),
            home + QStringLiteral("/.local/share/flatpak/exports/share/applications"),
            QStringLiteral("/var/lib/flatpak/exports/share/applications"),
            QStringLiteral("/usr/local/share/applications"),
            QStringLiteral("/usr/share/applications"),
        };
        for (const QString& d : dirs) {
            const QFileInfo di(d);
            if (!di.isDir() || di.isSymLink()) continue;
            QDir dir(d);
            const QStringList files = dir.entryList({QStringLiteral("*.desktop")}, QDir::Files, QDir::Name);
            for (const QString& fn : files) {
                const QString full = dir.filePath(fn);
                const QFileInfo fi(full);
                if (!fi.isFile() || fi.isSymLink() || fi.size() > 131072) continue;
                QFile f(full);
                if (!f.open(QIODevice::ReadOnly | QIODevice::Text)) continue;
                bool inEntry = false;
                QString dName, dExec, dIcon, dWm;
                while (!f.atEnd()) {
                    const QString s = QString::fromUtf8(f.readLine()).trimmed();
                    if (s.isEmpty() || s.startsWith(QLatin1Char('#'))) continue;
                    if (s.startsWith(QLatin1Char('['))) {
                        inEntry = (s == QLatin1String("[Desktop Entry]"));
                        continue;
                    }
                    if (!inEntry) continue;
                    const int eq = s.indexOf(QLatin1Char('='));
                    if (eq <= 0) continue;
                    const QString k = s.left(eq).trimmed();
                    const QString v = s.mid(eq + 1).trimmed();
                    if (k == QLatin1String("Name") && dName.isEmpty()) dName = v.toLower();
                    else if (k == QLatin1String("Exec") && dExec.isEmpty()) {
                        const QString tok = v.split(QLatin1Char(' '), Qt::SkipEmptyParts).value(0)
                                                .remove(QLatin1Char('"')).remove(QLatin1Char('\''));
                        dExec = QFileInfo(tok).fileName().toLower();
                    } else if (k == QLatin1String("Icon") && dIcon.isEmpty()) {
                        dIcon = v;
                    } else if (k == QLatin1String("StartupWMClass") && dWm.isEmpty()) {
                        dWm = v.toLower();
                    }
                }
                if (dIcon.isEmpty()) continue;
                const QString stem = fn.left(fn.size() - 8).toLower();
                if (!byName_.contains(stem)) byName_.insert(stem, dIcon);
                if (!dName.isEmpty() && !byName_.contains(dName)) byName_.insert(dName, dIcon);
                if (!dExec.isEmpty() && !byExec_.contains(dExec)) byExec_.insert(dExec, dIcon);
                if (!dWm.isEmpty() && !byWm_.contains(dWm)) byWm_.insert(dWm, dIcon);
            }
        }
    }

    bool indexed_ = false;
    QMap<QString, QString> byExec_;
    QMap<QString, QString> byName_;
    QMap<QString, QString> byWm_;
    QMap<QString, QPixmap> cache_;
};

// =========================================================================
// PeakBarWidget — Stereo L/R Output Peak Bar
// =========================================================================

class PeakBarWidget : public QWidget {
    Q_OBJECT
public:
    explicit PeakBarWidget(QWidget* parent = nullptr) : QWidget(parent) {
        setFixedHeight(8);
    }

    void setPeaks(float pkL, float pkR, const QColor& acc) {
        const float tgtL = std::clamp(pkL, 0.0f, 1.0f);
        const float tgtR = std::clamp(pkR, 0.0f, 1.0f);
        const auto nowTp = std::chrono::steady_clock::now();
        double dtSec = 0.016;
        if (lastTick_.time_since_epoch().count() != 0) {
            dtSec = std::clamp(
                std::chrono::duration<double>(nowTp - lastTick_).count(),
                0.001, 0.25);
        }
        lastTick_ = nowTp;
        const float aRise = 1.0f - std::exp(-static_cast<float>(dtSec / 0.010));
        const float aFall = 1.0f - std::exp(-static_cast<float>(dtSec / 0.065));
        auto step = [aRise, aFall](float prev, float tgt) -> float {
            float nxt = prev + ((tgt > prev) ? aRise : aFall) * (tgt - prev);
            if (std::abs(nxt - tgt) < 0.002f) nxt = tgt;
            return std::clamp(nxt, 0.0f, 1.0f);
        };
        const float nxtL = step(pkL_, tgtL);
        const float nxtR = step(pkR_, tgtR);
        if (std::abs(nxtL - pkL_) > 0.001f || std::abs(nxtR - pkR_) > 0.001f || accent_ != acc) {
            pkL_ = nxtL;
            pkR_ = nxtR;
            accent_ = acc;
            update();
        }
    }

protected:
    void paintEvent(QPaintEvent*) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        const double w = static_cast<double>(width());

        auto drawChan = [&](int row, float val) {
            const double y = static_cast<double>(row * 4);
            p.setPen(Qt::NoPen);
            p.setBrush(QColor(QStringLiteral("#0f172a")));
            p.drawRoundedRect(QRectF(0.0, y, w, 3.0), 1.5, 1.5);
            if (val > 0.005f) {
                const double fw = static_cast<double>(val) * w;
                QLinearGradient lg(0.0, 0.0, w, 0.0);
                lg.setColorAt(0.0, accent_);
                lg.setColorAt(0.75, QColor(QStringLiteral("#F59E0B")));
                lg.setColorAt(1.0, QColor(QStringLiteral("#EF4444")));
                p.setBrush(lg);
                p.drawRoundedRect(QRectF(0.0, y, fw, 3.0), 1.5, 1.5);
            }
        };
        drawChan(0, pkL_);
        drawChan(1, pkR_);
    }

private:
    float pkL_ = 0.0f;
    float pkR_ = 0.0f;
    std::chrono::steady_clock::time_point lastTick_{};
    QColor accent_{"#22c55e"};
};

// =========================================================================
// SwipeFader — Capsule Swipe Volume Fader
// =========================================================================

class SwipeFader : public QWidget {
    Q_OBJECT
public:
    explicit SwipeFader(
        const QString& label,
        const QColor& accent,
        bool compact = false,
        QWidget* parent = nullptr)
        : QWidget(parent), label_(label), accent_(accent), compact_(compact) {
        setCursor(Qt::PointingHandCursor);
        setFocusPolicy(Qt::StrongFocus);
        updateMetrics(100);
    }

    void updateMetrics(int fontPct) {
        fontPct_ = normalizeFontScalePct(fontPct);
        setFixedHeight(scaledPx(compact_ ? 28 : 34, fontPct_));
        update();
    }

    void setStepProvider(std::function<int()> fn) {
        stepFn_ = std::move(fn);
        const int s = stepFn_ ? stepFn_() : 5;
        setToolTip(QStringLiteral("Drag, mouse wheel, or Left/Right/Up/Down keys adjust by ±%1%").arg(s));
    }

    void setThemeColors(
        const QColor& accent,
        const QColor& trackBg,
        const QColor& trackBorder,
        const QColor& txtBright,
        const QColor& txtMuted) {
        accent_ = accent;
        trackBg_ = trackBg;
        trackBorder_ = trackBorder;
        txtBright_ = txtBright;
        txtMuted_ = txtMuted;
        update();
    }

    void setLabel(const QString& lbl) {
        if (label_ != lbl) {
            label_ = lbl;
            update();
        }
    }

    void setValue(int pct, bool muted) {
        const int cl = std::clamp(pct, 0, 150);
        if (value_ != cl || muted_ != muted) {
            if (!dragging_) value_ = cl;
            muted_ = muted;
            update();
        }
    }

    int value() const { return value_; }
    bool isDragging() const { return dragging_; }

signals:
    void valueChanged(int pct);
    void valueCommitted(int pct);

protected:
    void mousePressEvent(QMouseEvent* event) override {
        if (event->button() == Qt::LeftButton) {
            dragging_ = true;
            setFromPos(event->position().x());
            event->accept();
        }
    }

    void mouseMoveEvent(QMouseEvent* event) override {
        if (dragging_) {
            setFromPos(event->position().x());
            event->accept();
        }
    }

    void mouseReleaseEvent(QMouseEvent* event) override {
        if (dragging_ && event->button() == Qt::LeftButton) {
            dragging_ = false;
            setFromPos(event->position().x());
            emit valueCommitted(value_);
            event->accept();
        }
    }

    void wheelEvent(QWheelEvent* event) override {
        const int dx = event->angleDelta().x();
        const int dy = event->angleDelta().y();
        int delta = std::abs(dy) >= std::abs(dx) ? dy : dx;
        int notches = 1;
        if (delta != 0) {
            notches = std::max<int>(1, static_cast<int>(std::lround(std::abs(delta) / 120.0)));
        } else {
            const QPoint pd = event->pixelDelta();
            delta = std::abs(pd.y()) >= std::abs(pd.x()) ? pd.y() : pd.x();
            notches = std::max<int>(1, static_cast<int>(std::lround(std::abs(delta) / 24.0)));
        }
        if (delta != 0) {
            const int step = std::max(1, stepFn_ ? stepFn_() : 5) * notches;
            const int nxt = std::clamp(value_ + (delta > 0 ? step : -step), 0, 150);
            if (nxt != value_) {
                value_ = nxt;
                update();
                emit valueChanged(value_);
                emit valueCommitted(value_);
            }
            event->accept();
        }
    }

    void keyPressEvent(QKeyEvent* event) override {
        const int step = std::max(1, stepFn_ ? stepFn_() : 5);
        int units = 0;
        switch (event->key()) {
            case Qt::Key_Right:
            case Qt::Key_Up:
                units = 1;
                break;
            case Qt::Key_Left:
            case Qt::Key_Down:
                units = -1;
                break;
            case Qt::Key_PageUp:
                units = 2;
                break;
            case Qt::Key_PageDown:
                units = -2;
                break;
            default:
                QWidget::keyPressEvent(event);
                return;
        }
        const int nxt = std::clamp(value_ + units * step, 0, 150);
        if (nxt != value_) {
            value_ = nxt;
            update();
            emit valueChanged(value_);
            emit valueCommitted(value_);
        }
        event->accept();
    }

    void paintEvent(QPaintEvent*) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        const double w = static_cast<double>(width());
        const double h = static_cast<double>(height());
        const QRectF r(1.0, 1.0, w - 2.0, h - 2.0);
        const double rad = r.height() / 2.0;

        p.setBrush(trackBg_);
        p.setPen(QPen(dragging_ ? accent_ : trackBorder_, 1.2));
        p.drawRoundedRect(r, rad, rad);

        // Unity 100% tick mark (subtle vertical notch)
        const double ux = r.left() + (100.0 / 150.0) * r.width();
        p.setPen(QPen(QColor(255, 255, 255, 38), 1.0));
        p.drawLine(QPointF(ux, r.top() + 5.0), QPointF(ux, r.bottom() - 5.0));

        // Active fill pill clipped inside capsule groove
        const double frac = std::clamp(static_cast<double>(value_) / 150.0, 0.0, 1.0);
        const QColor fillCol = muted_ ? QColor(QStringLiteral("#64748b")) : accent_;
        if (frac > 0.005) {
            p.save();
            QPainterPath clip;
            clip.addRoundedRect(r, rad, rad);
            p.setClipPath(clip);
            const double fw = std::max(r.height() * 0.5, frac * r.width());
            QLinearGradient lg(r.left(), 0.0, r.left() + fw, 0.0);
            QColor c0 = fillCol;
            c0.setAlpha(compact_ ? 150 : 175);
            QColor c1 = fillCol;
            c1.setAlpha(230);
            lg.setColorAt(0.0, c0);
            lg.setColorAt(1.0, c1);
            p.setPen(Qt::NoPen);
            p.setBrush(lg);
            p.drawRoundedRect(QRectF(r.left(), r.top(), fw, r.height()), rad, rad);
            p.restore();

            // Circular thumb knob near fill edge
            const double tx = std::clamp(
                r.left() + frac * r.width(),
                r.left() + r.height() * 0.48,
                r.right() - r.height() * 0.48);
            const double tr = r.height() * (compact_ ? 0.30 : 0.33);
            p.setBrush(QColor(QStringLiteral("#F8FAFC")));
            p.setPen(QPen(fillCol, 2.0));
            p.drawEllipse(QPointF(tx, r.center().y()), tr, tr);
        }

        // Crisp label & percentage text with subtle shadow so text stays legible across the circular thumb
        QFont f(QStringLiteral("Inter"));
        f.setPixelSize(scaledPx(compact_ ? 11 : 12, fontPct_));
        f.setWeight(QFont::Bold);
        p.setFont(f);
        const QFontMetrics fm(f);
        const QRectF leftRect = r.adjusted(12.0, 0.0, -54.0, 0.0);
        const QString elidedLbl = fm.elidedText(label_, Qt::ElideRight, std::max(20, static_cast<int>(leftRect.width())));
        const QRectF rightRect = r.adjusted(10.0, 0.0, -12.0, 0.0);
        const QString pctText = muted_ ? QStringLiteral("MUTED") : QStringLiteral("%1%").arg(value_);

        p.setPen(QColor(10, 14, 20, 180));
        p.drawText(leftRect.translated(1.0, 1.0), Qt::AlignVCenter | Qt::AlignLeft, elidedLbl);
        p.drawText(rightRect.translated(1.0, 1.0), Qt::AlignVCenter | Qt::AlignRight, pctText);

        p.setPen(muted_ ? txtMuted_ : txtBright_);
        p.drawText(leftRect, Qt::AlignVCenter | Qt::AlignLeft, elidedLbl);
        p.drawText(rightRect, Qt::AlignVCenter | Qt::AlignRight, pctText);
    }

private:
    void setFromPos(double x) {
        const double pad = 6.0;
        const double usable = std::max(10.0, static_cast<double>(width()) - pad * 2.0);
        const double norm = std::clamp((x - pad) / usable, 0.0, 1.0);
        const int nxt = std::clamp(static_cast<int>(std::lround(norm * 150.0)), 0, 150);
        if (nxt != value_) {
            value_ = nxt;
            update();
            emit valueChanged(value_);
        }
    }

    QString label_;
    QColor accent_{"#22c55e"};
    QColor trackBg_{"#1b212a"};
    QColor trackBorder_{"#334155"};
    QColor txtBright_{"#f8fafc"};
    QColor txtMuted_{"#94a3b8"};
    bool compact_ = false;
    int fontPct_ = 100;
    int value_ = 80;
    bool muted_ = false;
    bool dragging_ = false;
    std::function<int()> stepFn_;
};

// =========================================================================
// SalsaOsdOverlay — Click-Through Plasma-Style Volume / Mute HUD
// =========================================================================

class SalsaOsdOverlay : public QWidget {
    Q_OBJECT
public:
    explicit SalsaOsdOverlay(QWidget* parent = nullptr) : QWidget(parent) {
        setWindowFlags(
            Qt::ToolTip | Qt::FramelessWindowHint | Qt::WindowStaysOnTopHint
            | Qt::WindowTransparentForInput | Qt::WindowDoesNotAcceptFocus);
        setAttribute(Qt::WA_TranslucentBackground, true);
        setAttribute(Qt::WA_ShowWithoutActivating, true);
        setAttribute(Qt::WA_TransparentForMouseEvents, true);

        hideTimer_ = new QTimer(this);
        hideTimer_->setSingleShot(true);
        connect(hideTimer_, &QTimer::timeout, this, &QWidget::hide);
    }

    void showVolume(
        int pct,
        bool muted,
        const QString& label,
        bool isMic,
        const QColor& accent,
        int fontPct,
        const QPoint& rawAnchorPt) {
        pct_ = std::clamp(pct, 0, 150);
        muted_ = muted;
        isMic_ = isMic;
        title_ = label.isEmpty() ? (isMic ? QStringLiteral("Microphone Input") : QStringLiteral("Master Output")) : label;
        accent_ = accent;
        fontPct_ = normalizeFontScalePct(fontPct);

        const int w = scaledPx(340, fontPct_);
        const int h = scaledPx(76, fontPct_);
        resize(w, h);

        QScreen* scr = screenForTrayPoint(rawAnchorPt, rawAnchorPt.x() != 0 || rawAnchorPt.y() != 0);
        QRect avail = scr ? scr->availableGeometry() : QRect(0, 0, 1920, 1080);
        if (avail.width() < 200 || avail.height() < 200) {
            avail = scr ? scr->geometry() : QRect(0, 0, 1920, 1080);
        }
        const int ox = avail.x() + (avail.width() - w) / 2;
        const int oy = avail.y() + avail.height() - h - scaledPx(62, fontPct_);
        move(ox, oy);
        show();
        salsa_apply_x11_popup_hints(static_cast<uint64_t>(winId()), true, true, ox, oy);
        update();
        hideTimer_->start(1650);
    }

protected:
    void drawAudioSymbol(QPainter& p, const QColor& accent) {
        const double cy = static_cast<double>(height()) / 2.0;
        const QColor strokeCol = muted_ ? QColor(QStringLiteral("#94A3B8")) : QColor(QStringLiteral("#CBD5E1"));
        const QColor waveCol = muted_ ? QColor(QStringLiteral("#EF4444")) : accent;

        p.save();
        if (!isMic_) {
            QPainterPath cone;
            cone.moveTo(19.0, cy - 4.8);
            cone.lineTo(24.5, cy - 4.8);
            cone.lineTo(31.5, cy - 11.2);
            cone.lineTo(31.5, cy + 11.2);
            cone.lineTo(24.5, cy + 4.8);
            cone.lineTo(19.0, cy + 4.8);
            cone.closeSubpath();

            p.setBrush(QColor(strokeCol.red(), strokeCol.green(), strokeCol.blue(), 28));
            p.setPen(QPen(strokeCol, 1.9, Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
            p.drawPath(cone);

            if (muted_) {
                p.setPen(QPen(waveCol, 2.1, Qt::SolidLine, Qt::RoundCap));
                p.drawLine(QPointF(36.0, cy - 4.5), QPointF(44.5, cy + 4.5));
                p.drawLine(QPointF(44.5, cy - 4.5), QPointF(36.0, cy + 4.5));
            } else {
                p.setPen(QPen(waveCol, 2.0, Qt::SolidLine, Qt::RoundCap));
                p.setBrush(Qt::NoBrush);
                if (pct_ >= 1) {
                    p.drawArc(QRectF(29.0, cy - 6.0, 11.0, 12.0), -46 * 16, 92 * 16);
                }
                if (pct_ >= 35) {
                    p.drawArc(QRectF(31.5, cy - 10.5, 16.0, 21.0), -48 * 16, 96 * 16);
                }
            }
        } else {
            p.setBrush(QColor(strokeCol.red(), strokeCol.green(), strokeCol.blue(), 28));
            p.setPen(QPen(strokeCol, 1.85, Qt::SolidLine, Qt::RoundCap));
            p.drawRoundedRect(QRectF(25.5, cy - 12.5, 8.0, 14.5), 4.0, 4.0);
            p.setBrush(Qt::NoBrush);
            p.setPen(QPen(waveCol, 1.9, Qt::SolidLine, Qt::RoundCap));
            p.drawArc(QRectF(22.5, cy - 7.5, 14.0, 14.0), -180 * 16, 180 * 16);
            p.drawLine(QPointF(29.5, cy + 6.5), QPointF(29.5, cy + 11.5));
            p.drawLine(QPointF(24.5, cy + 11.5), QPointF(34.5, cy + 11.5));
            if (muted_) {
                p.drawLine(QPointF(20.0, cy + 10.0), QPointF(39.0, cy - 10.0));
            }
        }
        p.restore();
    }

    void paintEvent(QPaintEvent*) override {
        QPainter p(this);
        p.setRenderHint(QPainter::Antialiasing, true);
        const double w = static_cast<double>(width());
        const double h = static_cast<double>(height());
        const QRectF r(1.0, 1.0, w - 2.0, h - 2.0);

        QLinearGradient bgGrad(0.0, 0.0, 0.0, h);
        bgGrad.setColorAt(0.0, QColor(34, 40, 50, 198));
        bgGrad.setColorAt(1.0, QColor(20, 24, 31, 212));
        p.setBrush(bgGrad);
        p.setPen(QPen(QColor(255, 255, 255, 42), 1.0));
        p.drawRoundedRect(r, 14.0, 14.0);

        p.setBrush(Qt::NoBrush);
        p.setPen(QPen(QColor(255, 255, 255, 15), 1.0));
        p.drawRoundedRect(QRectF(2.0, 2.0, w - 4.0, h - 4.0), 13.0, 13.0);

        drawAudioSymbol(p, accent_);

        const QRectF textRect(58.0, 11.0, w - 76.0, 24.0);
        QFont ft(QStringLiteral("Inter"));
        ft.setPixelSize(scaledPx(13, fontPct_));
        ft.setWeight(QFont::Medium);
        p.setFont(ft);
        p.setPen(muted_ ? QColor(QStringLiteral("#FCA5A5")) : QColor(241, 245, 249, 238));
        const QString rawLabel = muted_ ? QStringLiteral("Muted — %1").arg(title_)
                                        : QStringLiteral("%1% — %2").arg(pct_).arg(title_);
        const QString elided = QFontMetricsF(ft).elidedText(rawLabel, Qt::ElideRight, textRect.width());
        p.drawText(textRect, Qt::AlignVCenter | Qt::AlignLeft, elided);

        const QRectF bar(58.0, h - 24.0, w - 76.0, 12.0);
        p.setPen(Qt::NoPen);
        p.setBrush(QColor(255, 255, 255, 28));
        p.drawRoundedRect(bar, 5.0, 5.0);

        const double frac = muted_ ? 0.0 : std::clamp(static_cast<double>(pct_) / 100.0, 0.0, 1.0);
        if (frac > 0.0) {
            const double fillW = std::max(8.0, frac * bar.width());
            QLinearGradient lg(bar.left(), 0.0, bar.right(), 0.0);
            if (isMic_) {
                lg.setColorAt(0.0, accent_.darker(120));
                lg.setColorAt(1.0, accent_.lighter(115));
            } else {
                lg.setColorAt(0.0, accent_.darker(135));
                lg.setColorAt(0.65, accent_);
                lg.setColorAt(1.0, pct_ <= 100 ? accent_.lighter(118) : QColor(QStringLiteral("#F59E0B")));
            }
            p.setBrush(lg);
            p.drawRoundedRect(QRectF(bar.left(), bar.top(), fillW, bar.height()), 5.0, 5.0);
        }
    }

private:
    int pct_ = 80;
    bool muted_ = false;
    bool isMic_ = false;
    QString title_ = QStringLiteral("Salsa Master");
    QColor accent_{"#22c55e"};
    int fontPct_ = 100;
    QTimer* hideTimer_ = nullptr;
};

// =========================================================================
// PopupWindowFrame & TrayController
// =========================================================================

class PopupWindowFrame : public QFrame {
public:
    explicit PopupWindowFrame(std::function<void()> onDeactivate)
        : onDeactivate_(std::move(onDeactivate)) {
        setWindowFlags(Qt::Tool | Qt::FramelessWindowHint | Qt::WindowStaysOnTopHint);
        setAttribute(Qt::WA_TranslucentBackground, true);
    }

protected:
    void showEvent(QShowEvent* ev) override {
        QFrame::showEvent(ev);
        QTimer::singleShot(10, this, [this]() {
            if (isVisible()) {
                salsa_apply_x11_popup_hints(static_cast<uint64_t>(winId()), true, true, 0, 0);
            }
        });
    }
    void changeEvent(QEvent* ev) override {
        if (ev->type() == QEvent::ActivationChange && !isActiveWindow()) {
            if (QApplication::activePopupWidget() == nullptr && onDeactivate_) {
                onDeactivate_();
            }
        }
        QFrame::changeEvent(ev);
    }
    void keyPressEvent(QKeyEvent* ev) override {
        if (ev->key() == Qt::Key_Escape) {
            hide();
            ev->accept();
            return;
        }
        QFrame::keyPressEvent(ev);
    }

private:
    std::function<void()> onDeactivate_;
};

class TrayController : public QObject {
    Q_OBJECT
public:
    explicit TrayController(bool withSni = true, QObject* parent = nullptr);
    ~TrayController() override;

    static void onSniCallback(void* ctx, int kind, int xOrDelta, int y, const char* ori);

    void syncFromEngine(bool showOsdIfExternal = false);
    void popupToggle(const QPoint& rawPt);
    void showMenuAt(const QPoint& rawPt);
    void toggleMasterMute();
    void toggleMicMute();
    void handleScroll(int delta, const QString& orientation);

private:
    void buildPopup();
    void buildContextMenu();
    void applyThemeAndFont(bool saveFont = false);
    void positionAndShowPopup(const QPoint& rawPt);
    void keepBottomAnchored();
    void populateQuickMixer();
    void rebuildEqPresetCombo();
    void populateDeviceCombos();
    void updateRateComboLabels(const QString& pbId);
    void checkSettingsSync();
    void onSniDispatch(int kind, int xOrDelta, int y, const QString& ori);
    void setFontPct(int pct, bool persist);
    void selectTrayTheme(const QString& name);
    void runSystemdAction(const char* action);

    bool withSni_ = true;
    void* sniHandle_ = nullptr;
    QString iconThemeDir_;

    SalsaThemeData theme_;
    QString themeName_;
    QJsonObject guiSettings_;
    uint64_t lastSettingsMtimeNs_ = 0;
    int fontPct_ = 100;
    int volStepPct_ = 5;
    bool leftClickQuickMixer_ = true;
    bool trayGlow_ = true;
    bool pinned_ = false;
    bool syncingUi_ = false;

    SalsaEngineStatus status_{};
    SalsaDeviceList devices_{};
    bool engineOnline_ = false;
    QString lastDevSig_;
    QString lastPbForRates_;
    QList<int> supportedPbRates_;

    QPoint lastTrayRawPt_;
    qint64 popupOpenAtMs_ = 0;
    qint64 osdSuppressUntilMs_ = 0;
    qint64 eqHoldUntilMs_ = 0;
    qint64 rateHoldUntilMs_ = 0;
    qint64 capRateHoldUntilMs_ = 0;
    qint64 btHoldUntilMs_ = 0;
    qint64 spkHoldUntilMs_ = 0;
    qint64 lastSpkSendMs_ = 0;
    qint64 micHoldUntilMs_ = 0;
    qint64 lastMicSendMs_ = 0;
    qint64 lastDevPollMs_ = 0;
    qint64 lastSettingsPollMs_ = 0;
    QString lastHdrLeaf_;
    int lastHdrIconSz_ = 0;
    QString lastSniTitle_;
    int lastSniOnline_ = -1;
    int lastSniMuted_ = -1;
    int lastSniGlow_ = -1;
    int anchorPanelLoc_ = 4;
    int anchorBottom_ = 0;
    QRect anchorAvail_;

    QString lastBtAlias_;
    qint64 lastBtPollMs_ = 0;
    int btPct_ = -1;
    QString btAddr_;
    bool btResetting_ = false;

    int lastSpkVol_ = -1;
    bool lastSpkMute_ = false;
    int lastMicVol_ = -1;
    bool lastMicMute_ = false;
    bool osdInitDone_ = false;

    QTimer* pollTimer_ = nullptr;
    QTimer* osdWatchTimer_ = nullptr;
    SalsaOsdOverlay* osd_ = nullptr;
    AppIconResolver iconResolver_;

    QFrame* popupWin_ = nullptr;
    QFrame* card_ = nullptr;
    QLabel* hdrIcon_ = nullptr;
    QLabel* lblStatusPill_ = nullptr;
    QLabel* lblHwBadge_ = nullptr;
    QPushButton* btnPin_ = nullptr;
    QPushButton* btnTabDev_ = nullptr;
    QPushButton* btnTabApps_ = nullptr;
    FontScaleBar* fontBar_ = nullptr;
    QPushButton* btnVolStep_ = nullptr;
    QStackedWidget* pages_ = nullptr;

    QFrame* btPill_ = nullptr;
    QLabel* lblBtInfo_ = nullptr;
    QPushButton* btnBtRst_ = nullptr;

    QComboBox* cmbOutDev_ = nullptr;
    QComboBox* cmbMicDev_ = nullptr;
    SwipeFader* faderSpk_ = nullptr;
    QPushButton* btnSpkMute_ = nullptr;
    PeakBarWidget* peakBar_ = nullptr;
    SwipeFader* faderMic_ = nullptr;
    QPushButton* btnMicMute_ = nullptr;

    QComboBox* cmbQuickEqPreset_ = nullptr;
    QPushButton* btnEqBypass_ = nullptr;
    QComboBox* cmbRate_ = nullptr;
    QPushButton* btnPerDevEq_ = nullptr;
    QPushButton* btnLimiter_ = nullptr;
    QPushButton* btnMidiTest_ = nullptr;
    QPushButton* btnCrossfeed_ = nullptr;
    QComboBox* cmbBtCodec_ = nullptr;
    QPushButton* btnBtAbr_ = nullptr;
    QComboBox* cmbCapRate_ = nullptr;
    QComboBox* cmbTrayTheme_ = nullptr;

    QLabel* lblAppCount_ = nullptr;
    QScrollArea* appsScroll_ = nullptr;
    QVBoxLayout* appsLay_ = nullptr;
    QLabel* lblNoApps_ = nullptr;
    struct PopupAppRow {
        uint32_t sid = 0;
        QWidget* row = nullptr;
        QLabel* iconLbl = nullptr;
        SwipeFader* fader = nullptr;
        QPushButton* btnMute = nullptr;
        qint64 holdUntilMs = 0;
        qint64 lastSentMs = 0;
    };
    std::vector<PopupAppRow> appRows_;

    QMenu* ctxMenu_ = nullptr;
    QAction* actStatus_ = nullptr;
    QAction* actMuteSpk_ = nullptr;
    QAction* actMuteMic_ = nullptr;
    QAction* actPin_ = nullptr;
    QAction* actLeftMode_ = nullptr;
    QAction* actGlow_ = nullptr;
    QAction* actStartEng_ = nullptr;
    QAction* actRestartEng_ = nullptr;
    QAction* actStopEng_ = nullptr;
    QList<QAction*> themeActions_;
    QList<QAction*> fontActions_;
};

TrayController::TrayController(bool withSni, QObject* parent)
    : QObject(parent), withSni_(withSni) {
    guiSettings_ = loadGuiSettings();
    lastSettingsMtimeNs_ = salsa_gui_settings_mtime_ns();
    fontPct_ = normalizeFontScalePct(salsa_load_font_scale());
    volStepPct_ = normalizeVolStepPct(guiSettings_.value(QStringLiteral("volume_step_pct")).toInt(5));

    if (guiSettings_.contains(QStringLiteral("tray_left_click_popover"))) {
        leftClickQuickMixer_ = guiSettings_.value(QStringLiteral("tray_left_click_popover")).toBool(true);
    } else {
        leftClickQuickMixer_ = guiSettings_.value(QStringLiteral("tray_left_click_popup")).toBool(true);
    }
    if (guiSettings_.contains(QStringLiteral("tray_green_glow"))) {
        trayGlow_ = guiSettings_.value(QStringLiteral("tray_green_glow")).toBool(true);
    } else {
        trayGlow_ = guiSettings_.value(QStringLiteral("tray_glow_enabled")).toBool(true);
    }

    theme_ = loadThemeForApp(true);
    themeName_ = guiSettings_.value(QStringLiteral("tray_theme")).toString().trimmed();
    if (themeName_.isEmpty()) {
        if (guiSettings_.contains(QStringLiteral("tray_breeze_emerald"))) {
            themeName_ = guiSettings_.value(QStringLiteral("tray_breeze_emerald")).toBool(false)
                             ? QStringLiteral("Plasma Breeze Emerald")
                             : QStringLiteral("Follow Studio Theme");
        } else {
            themeName_ = QStringLiteral("Follow Studio Theme");
        }
    }

    osd_ = new SalsaOsdOverlay();
    buildPopup();
    buildContextMenu();
    applyThemeAndFont(false);

    if (withSni_) {
        char* dirPtr = nullptr;
        sniHandle_ = salsa_sni_start(&TrayController::onSniCallback, this, &dirPtr);
        if (dirPtr) {
            iconThemeDir_ = SalsaStr(dirPtr).toQString();
        }
        if (sniHandle_) {
            size_t pngLen = 0;
            uint8_t* rawPtr = salsa_tray_header_icon_png(sniHandle_, &pngLen);
            const SalsaBytes pngBytes(rawPtr, pngLen);
            const QByteArray raw = pngBytes.toByteArray();
            if (!raw.isEmpty()) {
                QPixmap pm;
                if (pm.loadFromData(raw, "PNG") && hdrIcon_) {
                    hdrIcon_->setPixmap(pm.scaled(24, 24, Qt::KeepAspectRatio, Qt::SmoothTransformation));
                }
            }
        }
    }

    syncFromEngine(false);

    pollTimer_ = new QTimer(this);
    pollTimer_->setTimerType(Qt::PreciseTimer);
    pollTimer_->setInterval(1200);
    connect(pollTimer_, &QTimer::timeout, this, [this]() {
        const qint64 now = nowMs();
        if (now - lastSettingsPollMs_ >= 450) {
            lastSettingsPollMs_ = now;
            checkSettingsSync();
        }
        syncFromEngine(false);
    });
    pollTimer_->start();

    osdWatchTimer_ = new QTimer(this);
    osdWatchTimer_->setInterval(260);
    connect(osdWatchTimer_, &QTimer::timeout, this, [this]() {
        syncFromEngine(true);
    });
    osdWatchTimer_->start();
}

TrayController::~TrayController() {
    if (pollTimer_) pollTimer_->stop();
    if (osdWatchTimer_) osdWatchTimer_->stop();
    if (sniHandle_) {
        salsa_sni_stop(sniHandle_);
        sniHandle_ = nullptr;
    }
    delete osd_;
    delete popupWin_;
    delete ctxMenu_;
}

void TrayController::onSniCallback(void* ctx, int kind, int xOrDelta, int y, const char* ori) {
    if (!ctx) return;
    auto* self = static_cast<TrayController*>(ctx);
    const QString oriCopy = ori ? QString::fromUtf8(ori) : QString();
    QMetaObject::invokeMethod(
        self, [self, kind, xOrDelta, y, oriCopy]() {
            self->onSniDispatch(kind, xOrDelta, y, oriCopy);
        },
        Qt::QueuedConnection);
}

void TrayController::onSniDispatch(int kind, int xOrDelta, int y, const QString& ori) {
    if (kind == 0) {
        lastTrayRawPt_ = QPoint(xOrDelta, y);
        if (leftClickQuickMixer_) popupToggle(lastTrayRawPt_);
        else spawnDetachedWaylandBinary(QStringLiteral("salsa-studio"), {QStringLiteral("--studio")});
    } else if (kind == 1) {
        lastTrayRawPt_ = QPoint(xOrDelta, y);
        showMenuAt(lastTrayRawPt_);
    } else if (kind == 2) {
        toggleMasterMute();
    } else if (kind == 3) {
        handleScroll(xOrDelta, ori);
    }
}

void TrayController::buildPopup() {
    popupWin_ = new PopupWindowFrame([this]() {
        if (!pinned_ && (nowMs() - popupOpenAtMs_ > 280)) {
            popupWin_->hide();
        }
    });
    popupWin_->setObjectName(QStringLiteral("TrayPopupOuter"));
    auto* outerLay = new QVBoxLayout(popupWin_);
    outerLay->setContentsMargins(0, 0, 0, 0);

    card_ = new QFrame(popupWin_);
    card_->setObjectName(QStringLiteral("MixerCard"));
    outerLay->addWidget(card_);

    auto* root = new QVBoxLayout(card_);
    root->setContentsMargins(14, 10, 14, 10);
    root->setSpacing(7);

    // Header Bar
    auto* hdr = new QHBoxLayout();
    hdr->setSpacing(7);
    hdrIcon_ = new QLabel(card_);
    hdrIcon_->setFixedSize(24, 24);
    hdr->addWidget(hdrIcon_);

    auto* lblTitle = new QLabel(QStringLiteral("Salsa"), card_);
    lblTitle->setObjectName(QStringLiteral("TitleBanner"));
    hdr->addWidget(lblTitle);

    lblStatusPill_ = new QLabel(QStringLiteral("ACTIVE"), card_);
    lblStatusPill_->setObjectName(QStringLiteral("StatusPill"));
    hdr->addWidget(lblStatusPill_);
    hdr->addStretch(1);

    lblHwBadge_ = new QLabel(QStringLiteral("48 kHz · 32b"), card_);
    lblHwBadge_->setObjectName(QStringLiteral("HwBadge"));
    hdr->addWidget(lblHwBadge_);

    btnPin_ = new QPushButton(QStringLiteral("Pin"), card_);
    btnPin_->setObjectName(QStringLiteral("HdrToggle"));
    btnPin_->setCheckable(true);
    btnPin_->setCursor(Qt::PointingHandCursor);
    btnPin_->setToolTip(QStringLiteral("Keep popover open while clicking other windows"));
    connect(btnPin_, &QPushButton::toggled, this, [this](bool on) {
        pinned_ = on;
        if (actPin_) actPin_->setChecked(on);
    });
    hdr->addWidget(btnPin_);

    auto* btnClose = new QPushButton(QStringLiteral("×"), card_);
    btnClose->setObjectName(QStringLiteral("HdrClose"));
    btnClose->setFixedSize(24, 24);
    btnClose->setCursor(Qt::PointingHandCursor);
    btnClose->setToolTip(QStringLiteral("Close Quick Mixer"));
    connect(btnClose, &QPushButton::clicked, popupWin_, &QWidget::hide);
    hdr->addWidget(btnClose);
    root->addLayout(hdr);

    // KDE Plasma Underline Tabs + Font Scale + Wheel Step
    auto* tabRow = new QHBoxLayout();
    tabRow->setSpacing(4);
    btnTabDev_ = new QPushButton(QStringLiteral("Devices"), card_);
    btnTabDev_->setObjectName(QStringLiteral("TabUnderline"));
    btnTabDev_->setCheckable(true);
    btnTabDev_->setChecked(true);
    btnTabDev_->setCursor(Qt::PointingHandCursor);

    btnTabApps_ = new QPushButton(QStringLiteral("Applications"), card_);
    btnTabApps_->setObjectName(QStringLiteral("TabUnderline"));
    btnTabApps_->setCheckable(true);
    btnTabApps_->setCursor(Qt::PointingHandCursor);

    auto* grp = new QButtonGroup(this);
    grp->setExclusive(true);
    grp->addButton(btnTabDev_, 0);
    grp->addButton(btnTabApps_, 1);
    connect(grp, &QButtonGroup::idClicked, this, [this](int id) {
        if (pages_) pages_->setCurrentIndex(id);
        applyThemeAndFont(false);
        keepBottomAnchored();
    });
    tabRow->addWidget(btnTabDev_);
    tabRow->addWidget(btnTabApps_);
    tabRow->addStretch(1);

    fontBar_ = new FontScaleBar(fontPct_, true, card_);
    connect(fontBar_, &FontScaleBar::fontScaleChanged, this, [this](int pct) {
        setFontPct(pct, true);
    });
    tabRow->addWidget(fontBar_);

    btnVolStep_ = new QPushButton(QStringLiteral("Step ±%1%").arg(volStepPct_), card_);
    btnVolStep_->setObjectName(QStringLiteral("StepPill"));
    btnVolStep_->setCursor(Qt::PointingHandCursor);
    btnVolStep_->setToolTip(QStringLiteral("Click to cycle mouse-wheel & keyboard volume step (1%, 2%, 5%, 10%)"));
    connect(btnVolStep_, &QPushButton::clicked, this, [this]() {
        int next = SUPPORTED_VOL_STEPS[0];
        for (size_t i = 0; i < 4; ++i) {
            if (SUPPORTED_VOL_STEPS[i] == volStepPct_) {
                next = SUPPORTED_VOL_STEPS[(i + 1) % 4];
                break;
            }
        }
        volStepPct_ = next;
        btnVolStep_->setText(QStringLiteral("Step ±%1%").arg(volStepPct_));
        updateGuiSettingsPatch(QJsonObject{{QStringLiteral("volume_step_pct"), volStepPct_}});
        if (faderSpk_) faderSpk_->setStepProvider([this]() { return volStepPct_; });
        if (faderMic_) faderMic_->setStepProvider([this]() { return volStepPct_; });
        for (const auto& ar : appRows_) ar.fader->setStepProvider([this]() { return volStepPct_; });
    });
    tabRow->addWidget(btnVolStep_);
    root->addLayout(tabRow);
    root->addWidget(makeHLine(card_));

    pages_ = new QStackedWidget(card_);
    root->addWidget(pages_, 1);

    // --- Page 0: Devices & Quick DSP ---
    auto* devPage = new QWidget(pages_);
    auto* devLay = new QVBoxLayout(devPage);
    devLay->setContentsMargins(0, 2, 0, 2);
    devLay->setSpacing(7);

    // Bluetooth Live Pill (auto-hidden when wired)
    btPill_ = new QFrame(devPage);
    btPill_->setObjectName(QStringLiteral("BtPill"));
    auto* btLay = new QHBoxLayout(btPill_);
    btLay->setContentsMargins(10, 5, 8, 5);
    btLay->setSpacing(8);
    lblBtInfo_ = new QLabel(QStringLiteral("Bluetooth Ready"), btPill_);
    lblBtInfo_->setWordWrap(false);
    btLay->addWidget(lblBtInfo_, 1);
    btnBtRst_ = new QPushButton(QStringLiteral("Reset BT"), btPill_);
    btnBtRst_->setObjectName(QStringLiteral("SmallBtn"));
    btnBtRst_->setCursor(Qt::PointingHandCursor);
    connect(btnBtRst_, &QPushButton::clicked, this, [this]() {
        if (btResetting_) return;
        btResetting_ = true;
        btnBtRst_->setEnabled(false);
        btnBtRst_->setText(QStringLiteral("Resetting..."));
        std::thread([this]() {
            const SalsaStr msg(salsa_bt_reset_sync());
            const QString msgStr = msg.toQString();
            QMetaObject::invokeMethod(this, [this, msgStr]() {
                btResetting_ = false;
                if (btnBtRst_) {
                    btnBtRst_->setEnabled(true);
                    btnBtRst_->setText(QStringLiteral("Reset BT"));
                }
                if (lblBtInfo_ && !msgStr.isEmpty()) lblBtInfo_->setText(msgStr);
                lastBtPollMs_ = 0;
                syncFromEngine(false);
            }, Qt::QueuedConnection);
        }).detach();
    });
    btLay->addWidget(btnBtRst_);
    btPill_->hide();
    devLay->addWidget(btPill_);

    // Playback Devices Section
    devLay->addWidget(makeSectionHeader(QStringLiteral("Playback Devices"), devPage));
    auto* outDevRow = new QHBoxLayout();
    outDevRow->setSpacing(8);
    auto* lblOutD = new QLabel(QStringLiteral("Output"), devPage);
    lblOutD->setObjectName(QStringLiteral("DimLabel"));
    lblOutD->setFixedWidth(scaledPx(46, fontPct_));
    cmbOutDev_ = new QComboBox(devPage);
    connect(cmbOutDev_, &QComboBox::activated, this, [this](int idx) {
        if (syncingUi_) return;
        const QString pbId = cmbOutDev_->itemData(idx).toString();
        const QString capId = cmbMicDev_ ? cmbMicDev_->currentData().toString() : QString();
        if (!pbId.isEmpty() || !capId.isEmpty()) {
            salsa_set_devices(
                pbId.isEmpty() ? nullptr : pbId.toUtf8().constData(),
                capId.isEmpty() ? nullptr : capId.toUtf8().constData());
            lastDevSig_.clear();
            QTimer::singleShot(120, this, [this]() { syncFromEngine(false); });
        }
    });
    outDevRow->addWidget(lblOutD);
    outDevRow->addWidget(cmbOutDev_, 1);
    devLay->addLayout(outDevRow);

    auto* spkCol = new QVBoxLayout();
    spkCol->setContentsMargins(0, 0, 0, 0);
    spkCol->setSpacing(4);
    auto* spkRow = new QHBoxLayout();
    spkRow->setSpacing(6);
    faderSpk_ = new SwipeFader(QStringLiteral("Master Output"), theme_.qcolor("accent_primary", "#00e676"), false, devPage);
    faderSpk_->setStepProvider([this]() { return volStepPct_; });
    connect(faderSpk_, &SwipeFader::valueChanged, this, [this](int v) {
        const qint64 now = nowMs();
        osdSuppressUntilMs_ = now + 900;
        spkHoldUntilMs_ = now + 450;
        if (now - lastSpkSendMs_ >= 35) {
            lastSpkSendMs_ = now;
            salsa_set_master_volume(v, status_.desktop_muted ? 1 : 0);
        }
    });
    connect(faderSpk_, &SwipeFader::valueCommitted, this, [this](int v) {
        const qint64 now = nowMs();
        osdSuppressUntilMs_ = now + 900;
        spkHoldUntilMs_ = now + 450;
        lastSpkSendMs_ = now;
        lastSpkVol_ = v;
        salsa_set_master_volume(v, status_.desktop_muted ? 1 : 0);
    });
    spkRow->addWidget(faderSpk_, 1);

    btnSpkMute_ = new QPushButton(QStringLiteral("Mute"), devPage);
    btnSpkMute_->setObjectName(QStringLiteral("MuteBtn"));
    btnSpkMute_->setFixedWidth(scaledPx(62, fontPct_));
    btnSpkMute_->setCursor(Qt::PointingHandCursor);
    connect(btnSpkMute_, &QPushButton::clicked, this, &TrayController::toggleMasterMute);
    spkRow->addWidget(btnSpkMute_);
    spkCol->addLayout(spkRow);

    peakBar_ = new PeakBarWidget(devPage);
    spkCol->addWidget(peakBar_);
    devLay->addLayout(spkCol);

    // Recording Devices Section
    devLay->addWidget(makeSectionHeader(QStringLiteral("Recording Devices"), devPage));
    auto* micSelRow = new QHBoxLayout();
    micSelRow->setSpacing(8);
    auto* lblMicD = new QLabel(QStringLiteral("Input"), devPage);
    lblMicD->setObjectName(QStringLiteral("DimLabel"));
    lblMicD->setFixedWidth(scaledPx(46, fontPct_));
    cmbMicDev_ = new QComboBox(devPage);
    connect(cmbMicDev_, &QComboBox::activated, this, [this](int idx) {
        if (syncingUi_) return;
        const QString capId = cmbMicDev_->itemData(idx).toString();
        const QString pbId = cmbOutDev_ ? cmbOutDev_->currentData().toString() : QString();
        if (!pbId.isEmpty() || !capId.isEmpty()) {
            salsa_set_devices(
                pbId.isEmpty() ? nullptr : pbId.toUtf8().constData(),
                capId.isEmpty() ? nullptr : capId.toUtf8().constData());
            lastDevSig_.clear();
            QTimer::singleShot(120, this, [this]() { syncFromEngine(false); });
        }
    });
    micSelRow->addWidget(lblMicD);
    micSelRow->addWidget(cmbMicDev_, 1);
    devLay->addLayout(micSelRow);

    auto* micRow = new QHBoxLayout();
    micRow->setSpacing(6);
    faderMic_ = new SwipeFader(QStringLiteral("Mic / Input Gain"), theme_.qcolor("accent_secondary", "#38bdf8"), false, devPage);
    faderMic_->setStepProvider([this]() { return volStepPct_; });
    connect(faderMic_, &SwipeFader::valueChanged, this, [this](int v) {
        const qint64 now = nowMs();
        osdSuppressUntilMs_ = now + 900;
        micHoldUntilMs_ = now + 450;
        if (now - lastMicSendMs_ >= 35) {
            lastMicSendMs_ = now;
            salsa_set_mic_volume(v, status_.desktop_mic_muted ? 1 : 0);
        }
    });
    connect(faderMic_, &SwipeFader::valueCommitted, this, [this](int v) {
        const qint64 now = nowMs();
        osdSuppressUntilMs_ = now + 900;
        micHoldUntilMs_ = now + 450;
        lastMicSendMs_ = now;
        lastMicVol_ = v;
        salsa_set_mic_volume(v, status_.desktop_mic_muted ? 1 : 0);
    });
    micRow->addWidget(faderMic_, 1);

    btnMicMute_ = new QPushButton(QStringLiteral("Mute"), devPage);
    btnMicMute_->setObjectName(QStringLiteral("MuteBtn"));
    btnMicMute_->setFixedWidth(scaledPx(62, fontPct_));
    btnMicMute_->setCursor(Qt::PointingHandCursor);
    connect(btnMicMute_, &QPushButton::clicked, this, &TrayController::toggleMicMute);
    micRow->addWidget(btnMicMute_);
    devLay->addLayout(micRow);

    // EQ & Engine SubCard (5 quick rows)
    devLay->addWidget(makeSectionHeader(QStringLiteral("EQ & Engine"), devPage));
    auto* dspFrame = new QFrame(devPage);
    dspFrame->setObjectName(QStringLiteral("SubCard"));
    dspFrame->setSizePolicy(QSizePolicy::Expanding, QSizePolicy::Minimum);
    auto* dspLay = new QVBoxLayout(dspFrame);
    dspLay->setContentsMargins(9, 7, 9, 7);
    dspLay->setSpacing(6);

    // Row 1: EQ Preset + Bypass
    auto* eqRow = new QHBoxLayout();
    eqRow->setSpacing(6);
    auto* lblEq = new QLabel(QStringLiteral("EQ Preset"), dspFrame);
    lblEq->setObjectName(QStringLiteral("DimLabel"));
    eqRow->addWidget(lblEq);
    cmbQuickEqPreset_ = new QComboBox(dspFrame);
    rebuildEqPresetCombo();
    connect(cmbQuickEqPreset_, &QComboBox::activated, this, [this](int idx) {
        if (syncingUi_) return;
        const QString tag = cmbQuickEqPreset_->itemData(idx).toString();
        SalsaEqConfig eq = status_.eq_cfg;
        SalsaParamEqExt ext = status_.param_eq;
        eqHoldUntilMs_ = nowMs() + 900;
        if (tag.startsWith(QLatin1String("builtin:"))) {
            if (salsa_apply_builtin_eq_preset(tag.mid(8).toUtf8().constData(), &eq, &ext) == 0) {
                status_.eq_cfg = eq;
                status_.param_eq = ext;
                salsa_set_parametric_eq(&eq, &ext);
                syncFromEngine(false);
            }
        } else if (tag.startsWith(QLatin1String("autoeq:"))) {
            const QString nm = tag.mid(7);
            const QJsonArray list = parseJsonArr(SalsaStr(salsa_saved_autoeq_list_json()).toQString());
            for (const QJsonValue& av : list) {
                if (av.toObject().value(QStringLiteral("name")).toString() == nm) {
                    if (salsa_apply_saved_autoeq(jsonCompact(av.toObject()).toUtf8().constData(), &eq, &ext) == 0) {
                        status_.eq_cfg = eq;
                        status_.param_eq = ext;
                        salsa_set_parametric_eq(&eq, &ext);
                        syncFromEngine(false);
                    }
                    break;
                }
            }
        }
    });
    eqRow->addWidget(cmbQuickEqPreset_, 1);

    btnEqBypass_ = new QPushButton(QStringLiteral("Bypass"), dspFrame);
    btnEqBypass_->setObjectName(QStringLiteral("TogglePill"));
    btnEqBypass_->setCheckable(true);
    btnEqBypass_->setCursor(Qt::PointingHandCursor);
    connect(btnEqBypass_, &QPushButton::clicked, this, [this](bool checked) {
        if (syncingUi_) return;
        eqHoldUntilMs_ = nowMs() + 700;
        status_.eq_cfg.enabled = checked ? 0 : 1;
        salsa_set_dynamic_eq(&status_.eq_cfg);
        syncFromEngine(false);
    });
    eqRow->addWidget(btnEqBypass_);
    dspLay->addLayout(eqRow);

    // Row 2: Master Rate + Per-Dev EQ + Limiter + Test MIDI
    auto* modeRow = new QHBoxLayout();
    modeRow->setSpacing(5);
    auto* lblSr = new QLabel(QStringLiteral("Rate"), dspFrame);
    lblSr->setObjectName(QStringLiteral("DimLabel"));
    modeRow->addWidget(lblSr);

    cmbRate_ = new QComboBox(dspFrame);
    for (int r : TRAY_SAMPLE_RATES) {
        cmbRate_->addItem(QStringLiteral("%1 kHz").arg(r / 1000.0, 0, 'g', 4), r);
    }
    connect(cmbRate_, &QComboBox::activated, this, [this](int idx) {
        if (syncingUi_) return;
        const int r = qvarInt(cmbRate_->itemData(idx), 48000);
        if (r > 0) {
            rateHoldUntilMs_ = nowMs() + 900;
            const uint32_t pf = status_.hw_period_frames > 0 ? status_.hw_period_frames : 128;
            const uint32_t bits = status_.hw_format_bits > 0 ? status_.hw_format_bits : 32;
            salsa_set_engine_config(static_cast<uint32_t>(r), pf, bits);
            QTimer::singleShot(250, this, [this]() { syncFromEngine(false); });
        }
    });
    modeRow->addWidget(cmbRate_);

    btnPerDevEq_ = new QPushButton(QStringLiteral("Per-Dev EQ"), dspFrame);
    btnPerDevEq_->setObjectName(QStringLiteral("TogglePill"));
    btnPerDevEq_->setCheckable(true);
    btnPerDevEq_->setCursor(Qt::PointingHandCursor);
    connect(btnPerDevEq_, &QPushButton::clicked, this, [](bool checked) {
        updateGuiSettingsPatch(QJsonObject{
            {QStringLiteral("eq_per_device_enabled"), checked},
            {QStringLiteral("eq_per_device"), checked},
        });
    });
    modeRow->addWidget(btnPerDevEq_);

    btnLimiter_ = new QPushButton(QStringLiteral("Limiter"), dspFrame);
    btnLimiter_->setObjectName(QStringLiteral("TogglePill"));
    btnLimiter_->setCheckable(true);
    btnLimiter_->setCursor(Qt::PointingHandCursor);
    connect(btnLimiter_, &QPushButton::clicked, this, [this](bool checked) {
        if (syncingUi_) return;
        eqHoldUntilMs_ = nowMs() + 700;
        status_.eq_cfg.limiter_enabled = checked ? 1 : 0;
        salsa_set_dynamic_eq(&status_.eq_cfg);
        syncFromEngine(false);
    });
    modeRow->addWidget(btnLimiter_);

    btnMidiTest_ = new QPushButton(QStringLiteral("MIDI"), dspFrame);
    btnMidiTest_->setObjectName(QStringLiteral("SmallBtn"));
    btnMidiTest_->setCursor(Qt::PointingHandCursor);
    btnMidiTest_->setToolTip(QStringLiteral("Send Test MIDI Note (C4)"));
    connect(btnMidiTest_, &QPushButton::clicked, this, []() {
        salsa_send_test_midi(60, 110);
    });
    modeRow->addWidget(btnMidiTest_);
    dspLay->addLayout(modeRow);

    // Row 3: Crossfeed + BT Codec + BT ABR
    auto* apoRow = new QHBoxLayout();
    apoRow->setSpacing(5);

    btnCrossfeed_ = new QPushButton(QStringLiteral("Crossfeed"), dspFrame);
    btnCrossfeed_->setObjectName(QStringLiteral("TogglePill"));
    btnCrossfeed_->setCheckable(true);
    btnCrossfeed_->setCursor(Qt::PointingHandCursor);
    btnCrossfeed_->setToolTip(QStringLiteral("Toggle Bauer headphone stereo crossfeed"));
    connect(btnCrossfeed_, &QPushButton::clicked, this, [this](bool checked) {
        if (syncingUi_) return;
        eqHoldUntilMs_ = nowMs() + 700;
        status_.param_eq.crossfeed_enabled = checked ? 1 : 0;
        salsa_set_parametric_eq(&status_.eq_cfg, &status_.param_eq);
        QTimer::singleShot(80, this, [this]() { syncFromEngine(false); });
    });
    apoRow->addWidget(btnCrossfeed_);

    cmbBtCodec_ = new QComboBox(dspFrame);
    cmbBtCodec_->addItem(QStringLiteral("BT: Auto"), 0);
    cmbBtCodec_->addItem(QStringLiteral("LDAC"), 1);
    cmbBtCodec_->addItem(QStringLiteral("aptX HD"), 2);
    cmbBtCodec_->addItem(QStringLiteral("aptX"), 3);
    cmbBtCodec_->addItem(QStringLiteral("AAC"), 4);
    cmbBtCodec_->addItem(QStringLiteral("SBC"), 5);
    auto pushBtCfg = [this]() {
        if (syncingUi_) return;
        btHoldUntilMs_ = nowMs() + 1000;
        SalsaBtCodecCfg cfg = status_.bt_codec_cfg;
        cfg.preferred_codec = static_cast<uint32_t>(qvarInt(cmbBtCodec_->currentData(), 0));
        cfg.wireless_abr_enabled = btnBtAbr_->isChecked() ? 1 : 0;
        salsa_set_bt_codec_config(&cfg);
        QTimer::singleShot(300, this, [this]() { syncFromEngine(false); });
    };
    connect(cmbBtCodec_, &QComboBox::activated, this, [pushBtCfg](int) { pushBtCfg(); });
    apoRow->addWidget(cmbBtCodec_, 1);

    btnBtAbr_ = new QPushButton(QStringLiteral("BT ABR"), dspFrame);
    btnBtAbr_->setObjectName(QStringLiteral("TogglePill"));
    btnBtAbr_->setCheckable(true);
    btnBtAbr_->setCursor(Qt::PointingHandCursor);
    btnBtAbr_->setToolTip(QStringLiteral("Toggle Wireless Adaptive Bitrate (LDAC/AAC)"));
    connect(btnBtAbr_, &QPushButton::clicked, this, [pushBtCfg](bool) { pushBtCfg(); });
    apoRow->addWidget(btnBtAbr_);
    dspLay->addLayout(apoRow);

    // Row 4: Mic Rate + Quick Mixer Theme
    auto* micThemeRow = new QHBoxLayout();
    micThemeRow->setSpacing(6);
    auto* lblMicSr = new QLabel(QStringLiteral("Mic Rate"), dspFrame);
    lblMicSr->setObjectName(QStringLiteral("DimLabel"));
    micThemeRow->addWidget(lblMicSr);

    cmbCapRate_ = new QComboBox(dspFrame);
    cmbCapRate_->addItem(QStringLiteral("Auto (Follow Master)"), 0);
    for (int r : TRAY_SAMPLE_RATES) {
        cmbCapRate_->addItem(QStringLiteral("%1 kHz").arg(r / 1000.0, 0, 'g', 4), r);
    }
    cmbCapRate_->setToolTip(QStringLiteral("Pin capture/microphone hardware clock independently of Master playback clock"));
    connect(cmbCapRate_, &QComboBox::activated, this, [this](int idx) {
        if (syncingUi_) return;
        const int r = qvarInt(cmbCapRate_->itemData(idx), 0);
        capRateHoldUntilMs_ = nowMs() + 900;
        salsa_set_capture_sample_rate(static_cast<uint32_t>(r));
        QTimer::singleShot(250, this, [this]() { syncFromEngine(false); });
    });
    micThemeRow->addWidget(cmbCapRate_, 1);

    auto* lblTheme = new QLabel(QStringLiteral("Theme"), dspFrame);
    lblTheme->setObjectName(QStringLiteral("DimLabel"));
    micThemeRow->addWidget(lblTheme);

    cmbTrayTheme_ = new QComboBox(dspFrame);
    for (const QString& tn : allThemeNames(true)) {
        cmbTrayTheme_->addItem(tn, tn);
    }
    connect(cmbTrayTheme_, &QComboBox::activated, this, [this](int idx) {
        if (syncingUi_) return;
        const QString choice = cmbTrayTheme_->itemData(idx).toString().trimmed();
        if (!choice.isEmpty()) selectTrayTheme(choice);
    });
    micThemeRow->addWidget(cmbTrayTheme_, 1);
    dspLay->addLayout(micThemeRow);

    devLay->addWidget(dspFrame);
    devLay->addStretch(1);
    pages_->addWidget(devPage);

    // --- Page 1: Active Applications ---
    auto* appsPage = new QWidget(pages_);
    auto* appsOuter = new QVBoxLayout(appsPage);
    appsOuter->setContentsMargins(0, 2, 0, 2);
    appsOuter->setSpacing(8);

    auto* appsHdr = new QHBoxLayout();
    auto* lblApps = new QLabel(QStringLiteral("ACTIVE APP STREAMS"), appsPage);
    lblApps->setObjectName(QStringLiteral("SectionCap"));
    appsHdr->addWidget(lblApps);
    appsHdr->addStretch(1);
    lblAppCount_ = new QLabel(QStringLiteral("0 active"), appsPage);
    lblAppCount_->setObjectName(QStringLiteral("DimLabel"));
    appsHdr->addWidget(lblAppCount_);
    appsOuter->addLayout(appsHdr);

    appsScroll_ = new QScrollArea(appsPage);
    appsScroll_->setWidgetResizable(true);
    appsScroll_->setFrameShape(QFrame::NoFrame);
    appsScroll_->setMinimumHeight(200);
    appsScroll_->setMaximumHeight(320);
    auto* appsHost = new QWidget(appsScroll_);
    appsLay_ = new QVBoxLayout(appsHost);
    appsLay_->setContentsMargins(0, 0, 2, 0);
    appsLay_->setSpacing(6);
    lblNoApps_ = new QLabel(QStringLiteral("No active audio streams right now"), appsHost);
    lblNoApps_->setObjectName(QStringLiteral("DimLabel"));
    lblNoApps_->setAlignment(Qt::AlignCenter);
    lblNoApps_->setWordWrap(true);
    appsLay_->addWidget(lblNoApps_);
    appsLay_->addStretch(1);
    appsScroll_->setWidget(appsHost);
    appsOuter->addWidget(appsScroll_, 1);
    pages_->addWidget(appsPage);

    // Footer Quick Launch Row
    root->addWidget(makeHLine(card_));
    auto* foot = new QHBoxLayout();
    foot->setSpacing(6);
    auto* btnStudio = new QPushButton(QStringLiteral("Open Studio"), card_);
    btnStudio->setObjectName(QStringLiteral("PrimaryBtn"));
    btnStudio->setCursor(Qt::PointingHandCursor);
    connect(btnStudio, &QPushButton::clicked, this, [this]() {
        if (!pinned_) popupWin_->hide();
        spawnDetachedWaylandBinary(QStringLiteral("salsa-studio"), {QStringLiteral("--studio")});
    });
    foot->addWidget(btnStudio, 1);

    auto* btnGraph = new QPushButton(QStringLiteral("Graph"), card_);
    btnGraph->setObjectName(QStringLiteral("FooterBtn"));
    btnGraph->setCursor(Qt::PointingHandCursor);
    connect(btnGraph, &QPushButton::clicked, this, [this]() {
        if (!pinned_) popupWin_->hide();
        spawnDetachedWaylandBinary(QStringLiteral("salsa-studio"), {QStringLiteral("--topology")});
    });
    foot->addWidget(btnGraph);

    auto* btnDbg = new QPushButton(QStringLiteral("Debug"), card_);
    btnDbg->setObjectName(QStringLiteral("FooterBtn"));
    btnDbg->setCursor(Qt::PointingHandCursor);
    connect(btnDbg, &QPushButton::clicked, this, [this]() {
        if (!pinned_) popupWin_->hide();
        spawnDetachedWaylandBinary(QStringLiteral("salsa-debug"));
    });
    foot->addWidget(btnDbg);

    auto* btnRst = new QPushButton(QStringLiteral("Restart"), card_);
    btnRst->setObjectName(QStringLiteral("WarnBtn"));
    btnRst->setCursor(Qt::PointingHandCursor);
    connect(btnRst, &QPushButton::clicked, this, [this]() {
        runSystemdAction("restart");
    });
    foot->addWidget(btnRst);
    root->addLayout(foot);
}

void TrayController::buildContextMenu() {
    ctxMenu_ = new QMenu();
    actStatus_ = ctxMenu_->addAction(QStringLiteral("Salsa — Checking..."));
    actStatus_->setEnabled(false);
    ctxMenu_->addSeparator();

    auto* aPopover = ctxMenu_->addAction(QStringLiteral("Open Quick Mixer"));
    connect(aPopover, &QAction::triggered, this, [this]() {
        positionAndShowPopup(lastTrayRawPt_);
    });

    auto* aStudio = ctxMenu_->addAction(QStringLiteral("Open Salsa Studio"));
    connect(aStudio, &QAction::triggered, this, []() {
        spawnDetachedWaylandBinary(QStringLiteral("salsa-studio"), {QStringLiteral("--studio")});
    });

    auto* aGraph = ctxMenu_->addAction(QStringLiteral("Signal Flow Graph"));
    connect(aGraph, &QAction::triggered, this, []() {
        spawnDetachedWaylandBinary(QStringLiteral("salsa-studio"), {QStringLiteral("--topology")});
    });

    auto* aDbg = ctxMenu_->addAction(QStringLiteral("Live Debugger"));
    connect(aDbg, &QAction::triggered, this, []() {
        spawnDetachedWaylandBinary(QStringLiteral("salsa-debug"));
    });

    ctxMenu_->addSeparator();
    actMuteSpk_ = ctxMenu_->addAction(QStringLiteral("Mute Master Output"));
    connect(actMuteSpk_, &QAction::triggered, this, &TrayController::toggleMasterMute);

    actMuteMic_ = ctxMenu_->addAction(QStringLiteral("Mute Mic Input"));
    connect(actMuteMic_, &QAction::triggered, this, &TrayController::toggleMicMute);

    actPin_ = ctxMenu_->addAction(QStringLiteral("Pin Quick Mixer Open"));
    actPin_->setCheckable(true);
    connect(actPin_, &QAction::toggled, this, [this](bool on) {
        pinned_ = on;
        if (btnPin_) btnPin_->setChecked(on);
    });

    ctxMenu_->addSeparator();
    actGlow_ = ctxMenu_->addAction(QStringLiteral("Green Glow on Active Icon"));
    actGlow_->setCheckable(true);
    actGlow_->setChecked(trayGlow_);
    connect(actGlow_, &QAction::toggled, this, [this](bool on) {
        trayGlow_ = on;
        updateGuiSettingsPatch(QJsonObject{
            {QStringLiteral("tray_green_glow"), on},
            {QStringLiteral("tray_glow_enabled"), on},
        });
        syncFromEngine(false);
    });

    actLeftMode_ = ctxMenu_->addAction(QStringLiteral("Left-Click Opens Quick Mixer"));
    actLeftMode_->setCheckable(true);
    actLeftMode_->setChecked(leftClickQuickMixer_);
    connect(actLeftMode_, &QAction::toggled, this, [this](bool on) {
        leftClickQuickMixer_ = on;
        updateGuiSettingsPatch(QJsonObject{
            {QStringLiteral("tray_left_click_popover"), on},
            {QStringLiteral("tray_left_click_popup"), on},
        });
    });

    auto* themeSub = ctxMenu_->addMenu(QStringLiteral("Quick Mixer Theme"));
    auto* themeGrp = new QActionGroup(themeSub);
    themeGrp->setExclusive(true);
    themeActions_.clear();
    for (const QString& tn : allThemeNames(true)) {
        auto* ta = themeSub->addAction(tn);
        ta->setCheckable(true);
        ta->setData(tn);
        ta->setChecked(tn == themeName_);
        themeGrp->addAction(ta);
        themeActions_.append(ta);
        connect(ta, &QAction::triggered, this, [this, tn]() {
            selectTrayTheme(tn);
        });
    }

    auto* fontSub = ctxMenu_->addMenu(QStringLiteral("UI Font Scale"));
    auto* fontGrp = new QActionGroup(fontSub);
    fontGrp->setExclusive(true);
    fontActions_.clear();
    for (int pct : TRAY_FONT_STEPS) {
        auto* fa = fontSub->addAction(QStringLiteral("%1%").arg(pct));
        fa->setCheckable(true);
        fa->setData(pct);
        fa->setChecked(pct == fontPct_);
        fontGrp->addAction(fa);
        fontActions_.append(fa);
        connect(fa, &QAction::triggered, this, [this, pct]() {
            setFontPct(pct, true);
        });
    }

    ctxMenu_->addSeparator();
    actStartEng_ = ctxMenu_->addAction(QStringLiteral("Start Engine"));
    connect(actStartEng_, &QAction::triggered, this, [this]() { runSystemdAction("start"); });
    actRestartEng_ = ctxMenu_->addAction(QStringLiteral("Restart Engine"));
    connect(actRestartEng_, &QAction::triggered, this, [this]() { runSystemdAction("restart"); });
    actStopEng_ = ctxMenu_->addAction(QStringLiteral("Stop Engine"));
    connect(actStopEng_, &QAction::triggered, this, [this]() { runSystemdAction("stop"); });

    ctxMenu_->addSeparator();
    auto* aQuit = ctxMenu_->addAction(QStringLiteral("Quit Tray"));
    connect(aQuit, &QAction::triggered, qApp, &QApplication::quit);
}

void TrayController::runSystemdAction(const char* action) {
    const QByteArray actBytes(action ? action : "restart");
    std::thread([this, actBytes]() {
        salsa_ctl_systemd("lwasapi-audiodg.service", actBytes.constData());
        QMetaObject::invokeMethod(this, [this]() {
            lastDevSig_.clear();
            QTimer::singleShot(300, this, [this]() { syncFromEngine(false); });
        }, Qt::QueuedConnection);
    }).detach();
}

void TrayController::setFontPct(int pct, bool persist) {
    fontPct_ = normalizeFontScalePct(pct);
    if (persist) {
        salsa_save_font_scale(fontPct_);
        lastSettingsMtimeNs_ = salsa_gui_settings_mtime_ns();
    }
    applyThemeAndFont(false);
    if (popupWin_ && popupWin_->isVisible()) {
        positionAndShowPopup(lastTrayRawPt_);
    }
}

void TrayController::selectTrayTheme(const QString& name) {
    themeName_ = name;
    updateGuiSettingsPatch(QJsonObject{
        {QStringLiteral("tray_theme"), name},
        {QStringLiteral("tray_breeze_emerald"), name == QLatin1String("Plasma Breeze Emerald")},
    });
    theme_ = loadThemeForApp(true);
    applyThemeAndFont(false);
}

void TrayController::rebuildEqPresetCombo() {
    if (!cmbQuickEqPreset_) return;
    cmbQuickEqPreset_->blockSignals(true);
    cmbQuickEqPreset_->clear();
    const QJsonArray bArr = parseJsonArr(SalsaStr(salsa_builtin_eq_presets_json()).toQString());
    for (const QJsonValue& pv : bArr) {
        const QString nm = pv.toObject().value(QStringLiteral("name")).toString();
        if (!nm.isEmpty()) cmbQuickEqPreset_->addItem(nm, QStringLiteral("builtin:") + nm);
    }
    const QJsonArray aArr = parseJsonArr(SalsaStr(salsa_saved_autoeq_list_json()).toQString());
    for (const QJsonValue& av : aArr) {
        const QString nm = av.toObject().value(QStringLiteral("name")).toString();
        if (!nm.isEmpty()) cmbQuickEqPreset_->addItem(QStringLiteral("[AutoEQ] %1").arg(nm), QStringLiteral("autoeq:") + nm);
    }
    cmbQuickEqPreset_->blockSignals(false);
}

void TrayController::applyThemeAndFont(bool /*saveFont*/) {
    auto* app = qobject_cast<QApplication*>(QApplication::instance());
    if (app) {
        applyGlobalFontScale(app, fontPct_);
        app->setPalette(buildSalsaPalette(theme_));
    }

    auto sp = [this](int px) { return scaledPx(px, fontPct_); };

    if (cmbTrayTheme_) {
        syncingUi_ = true;
        const int idx = cmbTrayTheme_->findData(themeName_);
        if (idx >= 0) cmbTrayTheme_->setCurrentIndex(idx);
        syncingUi_ = false;
    }
    for (QAction* ta : themeActions_) {
        ta->setChecked(ta->data().toString() == themeName_);
    }
    for (QAction* fa : fontActions_) {
        fa->setChecked(fa->data().toInt() == fontPct_);
    }

    if (btnSpkMute_) btnSpkMute_->setFixedWidth(sp(62));
    if (btnMicMute_) btnMicMute_->setFixedWidth(sp(62));

    if (popupWin_) {
        QString qss = buildSalsaStylesheet(theme_, false, fontPct_);
        const QString bgCard = theme_.color("bg_card", "#181b20");
        const QString bgDeep = theme_.color("bg_deep", "#0c0e11");
        const QString bgCtrl = theme_.color("bg_control", "#232830");
        const QString bgHov = theme_.color("bg_hover", "#2d343f");
        const QString bdrCard = theme_.color("border_card", "#262b33");
        const QString bdrCtrl = theme_.color("border_control", "#334155");
        const QString txt = theme_.color("text_primary", "#e2e8f0");
        const QString brt = theme_.color("text_bright", "#f8fafc");
        const QString mut = theme_.color("text_muted", "#94a3b8");
        const QString acc = theme_.color("accent_primary", "#00e676");
        const QString warn = theme_.color("accent_warn", "#f59e0b");
        const QColor accCol = theme_.qcolor("accent_primary", "#00e676");
        const QColor secCol = theme_.qcolor("accent_secondary", "#38bdf8");
        const QString btBg = QStringLiteral("rgba(%1,%2,%3,30)").arg(secCol.red()).arg(secCol.green()).arg(secCol.blue());
        const QString btBd = QStringLiteral("rgba(%1,%2,%3,125)").arg(secCol.red()).arg(secCol.green()).arg(secCol.blue());
        const QString hwBg = QStringLiteral("rgba(%1,%2,%3,34)").arg(accCol.red()).arg(accCol.green()).arg(accCol.blue());
        const QString hwBd = QStringLiteral("rgba(%1,%2,%3,110)").arg(accCol.red()).arg(accCol.green()).arg(accCol.blue());
        const QString stBg = QStringLiteral("rgba(%1,%2,%3,38)").arg(accCol.red()).arg(accCol.green()).arg(accCol.blue());

        qss += QStringLiteral(
                   " QFrame#MixerCard { background-color: %1; border: 1px solid %2; border-radius: %3px; }"
                   " QFrame#SubCard { background-color: %4; border: 1px solid %5; border-radius: %6px; }"
                   " QFrame#HLine { background-color: %5; border: none; min-height: 1px; max-height: 1px; }"
                   " QFrame#BtPill { background-color: %15; border: 1px solid %16; border-radius: %7px; }"
                   " QLabel#DimLabel { color: %8; font-size: %9px; font-weight: 600; }"
                   " QLabel#SectionTitle { color: %8; font-size: %9px; font-weight: 700; }"
                   " QLabel#SectionCap { color: %8; font-size: %10px; font-weight: 700; letter-spacing: 0.7px; }"
                   " QLabel#HwBadge { background-color: %17; border: 1px solid %18;"
                   " border-radius: %11px; padding: %12; font-size: %9px; color: %13; font-weight: 700; }"
                   " QLabel#StatusPill { background-color: %19; color: %13; border: 1px solid %13;"
                   " border-radius: %11px; padding: %14; font-size: %10px; font-weight: 700; }")
                   .arg(bgCard, bdrCtrl, QString::number(sp(11)), bgDeep, bdrCard,
                        QString::number(sp(10)), QString::number(sp(10)), mut,
                        QString::number(sp(11)), QString::number(sp(10)),
                        QString::number(sp(8)),
                        QStringLiteral("%1px %2px").arg(sp(3)).arg(sp(8)),
                        acc,
                        QStringLiteral("%1px %2px").arg(sp(2)).arg(sp(7)),
                        btBg, btBd, hwBg, hwBd, stBg);

        qss += QStringLiteral(
                   " QPushButton#TabUnderline { background: transparent; color: %1; border: none;"
                   " border-bottom: 2px solid transparent; padding: %2; font-size: %3px; font-weight: 600; }"
                   " QPushButton#TabUnderline:hover { color: %4; }"
                   " QPushButton#TabUnderline:checked { color: %5; border-bottom: 2px solid %6; font-weight: 800; }"
                   " QPushButton#MuteBtn, QPushButton#SmallBtn, QPushButton#FooterBtn, QPushButton#TogglePill {"
                   " background-color: %7; color: %4; border: 1px solid %8; border-radius: %9px;"
                   " padding: %10; min-height: %11px; font-size: %12px; font-weight: 600; }"
                   " QPushButton#MuteBtn:hover, QPushButton#SmallBtn:hover, QPushButton#FooterBtn:hover, QPushButton#TogglePill:hover {"
                   " background-color: %13; border-color: %6; }"
                   " QPushButton#TogglePill:checked { background-color: %13; border-color: %6; color: %6; font-weight: 700; }"
                   " QPushButton#HdrToggle, QPushButton#HdrClose { background-color: %7; color: %1;"
                   " border: 1px solid %8; border-radius: %14px; padding: %15; font-size: %16px; font-weight: 700; }"
                   " QPushButton#HdrToggle:checked { background-color: %13; border-color: %6; color: %6; }"
                   " QPushButton#StepPill { background-color: %17; color: %6; border: 1px solid %8;"
                   " border-radius: %14px; padding: %15; font-size: %12px; font-weight: 700; }"
                   " QPushButton#WarnBtn { background-color: %7; color: %18; border: 1px solid %8;"
                   " border-radius: %9px; padding: %10; min-height: %11px; font-size: %12px; font-weight: 600; }"
                   " QPushButton#WarnBtn:hover { border-color: %18; background-color: %13; }")
                   .arg(mut,
                        QStringLiteral("%1px %2px").arg(sp(4)).arg(sp(10)),
                        QString::number(sp(12)),
                        txt, brt, acc, bgCtrl, bdrCtrl,
                        QString::number(sp(7)),
                        QStringLiteral("%1px %2px").arg(sp(3)).arg(sp(8)),
                        QString::number(sp(18)),
                        QString::number(sp(11)),
                        bgHov,
                        QString::number(sp(6)),
                        QStringLiteral("%1px %2px").arg(sp(2)).arg(sp(7)),
                        QString::number(sp(10)),
                        bgDeep,
                        warn);

        popupWin_->setStyleSheet(qss);
    }

    if (ctxMenu_) {
        ctxMenu_->setStyleSheet(
            QStringLiteral("QMenu { background-color: %1; color: %2; border: 1px solid %3; border-radius: 8px; padding: 4px; font-size: %4px; }"
                           " QMenu::item { padding: 6px 16px; border-radius: 5px; }"
                           " QMenu::item:selected { background-color: %5; color: %6; }"
                           " QMenu::item:disabled { color: %7; }")
                .arg(theme_.color("bg_card", "#181b20"),
                     theme_.color("text_primary", "#e2e8f0"),
                     theme_.color("border_control", "#334155"),
                     QString::number(sp(12)),
                     theme_.color("combo_popup_sel_bg", "#1e3a2f"),
                     theme_.color("combo_popup_sel_text", "#34d399"),
                     theme_.color("text_muted", "#94a3b8")));
    }

    const QColor trBg = theme_.qcolor("bg_deep", "#0c0e11");
    const QColor trBd = theme_.qcolor("border_control", "#334155");
    const QColor brt = theme_.qcolor("text_bright", "#f8fafc");
    const QColor mut = theme_.qcolor("text_muted", "#94a3b8");
    if (faderSpk_) {
        faderSpk_->updateMetrics(fontPct_);
        faderSpk_->setThemeColors(theme_.qcolor("accent_primary", "#00e676"), trBg, trBd, brt, mut);
    }
    if (faderMic_) {
        faderMic_->updateMetrics(fontPct_);
        faderMic_->setThemeColors(theme_.qcolor("accent_secondary", "#38bdf8"), trBg, trBd, brt, mut);
    }
    for (const auto& ar : appRows_) {
        ar.fader->updateMetrics(fontPct_);
        ar.fader->setThemeColors(theme_.qcolor("accent_secondary", "#38bdf8"), trBg, trBd, brt, mut);
    }
    if (fontBar_) fontBar_->setFontScalePct(fontPct_);
}

void TrayController::positionAndShowPopup(const QPoint& rawPt) {
    if (!popupWin_) return;
    populateQuickMixer();

    const int popW = std::clamp(scaledPx(456, fontPct_), 436, 580);
    popupWin_->setMinimumWidth(popW);
    popupWin_->setMaximumWidth(popW);
    popupWin_->resize(popW, popupWin_->sizeHint().height());
    popupWin_->adjustSize();
    int popH = std::clamp(popupWin_->sizeHint().height(), scaledPx(460, fontPct_), scaledPx(740, fontPct_));

    const bool hasRaw = (rawPt.x() > 0 || rawPt.y() > 0);
    const QPoint qtPt = hasRaw ? waylandToQtPoint(rawPt.x(), rawPt.y()) : QCursor::pos();
    QScreen* scr = screenForTrayPoint(rawPt, hasRaw);
    QRect avail = scr ? scr->availableGeometry() : QRect(0, 0, 1920, 1032);
    if (avail.width() < 200 || avail.height() < 200) {
        avail = scr ? scr->geometry() : QRect(0, 0, 1920, 1080);
    }
    popH = std::min(popH, std::max(360, avail.height() - 24));
    popupWin_->resize(popW, popH);

    int loc = 4, clearance = 48, rightPad = 24;
    salsa_plasma_panel_info(&loc, &clearance, &rightPad);
    const double dpr = scr ? std::max(1.0, static_cast<double>(scr->devicePixelRatio())) : 1.0;
    const int logicalPad = static_cast<int>(std::lround(rightPad / dpr));

    anchorPanelLoc_ = loc;
    anchorAvail_ = avail;

    int px = hasRaw ? (qtPt.x() - popW / 2) : (avail.right() + 1 - popW - std::max(16, logicalPad));
    px = std::clamp(px, avail.left() + 8, avail.right() + 1 - popW - 8);
    int py = (loc == 3) ? (avail.top() + 8) : (avail.bottom() + 1 - popH - 8);
    py = std::clamp(py, avail.top() + 8, avail.bottom() + 1 - popH - 8);
    anchorBottom_ = avail.bottom() + 1 - 8;

    popupWin_->move(px, py);
    popupOpenAtMs_ = nowMs();
    popupWin_->show();
    if (pollTimer_) pollTimer_->setInterval(33);
    salsa_apply_x11_popup_hints(static_cast<uint64_t>(popupWin_->winId()), true, true, 0, 0);
    popupWin_->raise();
    popupWin_->activateWindow();
}

void TrayController::keepBottomAnchored() {
    if (!popupWin_ || !popupWin_->isVisible()) return;
    const QList<QLayout*> lays = popupWin_->findChildren<QLayout*>();
    for (auto it = lays.crbegin(); it != lays.crend(); ++it) {
        (*it)->invalidate();
    }
    if (card_) card_->updateGeometry();
    if (popupWin_->layout()) popupWin_->layout()->activate();
    const QSize sh = popupWin_->sizeHint();
    const int popW = std::clamp(scaledPx(456, fontPct_), 436, 580);
    const int maxH = anchorAvail_.isValid() ? std::max(360, anchorAvail_.height() - 24) : 740;
    const int popH = std::min(maxH, std::max(scaledPx(360, fontPct_), sh.height()));
    if (std::abs(popH - popupWin_->height()) >= 2 || popW != popupWin_->width()) {
        popupWin_->resize(popW, popH);
        if (anchorPanelLoc_ != 3 && anchorAvail_.isValid()) {
            const int topLim = anchorAvail_.top() + 8;
            const int newY = std::max(topLim, anchorBottom_ - popH);
            popupWin_->move(popupWin_->x(), newY);
        }
    }
}

void TrayController::popupToggle(const QPoint& rawPt) {
    if (!popupWin_) return;
    if (popupWin_->isVisible()) {
        popupWin_->hide();
        if (pollTimer_) pollTimer_->setInterval(1200);
    } else {
        positionAndShowPopup(rawPt);
    }
}

void TrayController::showMenuAt(const QPoint& rawPt) {
    if (!ctxMenu_) return;
    syncFromEngine(false);
    if (rawPt.x() <= 0 && rawPt.y() <= 0) {
        ctxMenu_->popup(QCursor::pos());
    } else {
        ctxMenu_->popup(waylandToQtPoint(rawPt.x(), rawPt.y()));
    }
}

void TrayController::toggleMasterMute() {
    if (!engineOnline_) return;
    osdSuppressUntilMs_ = nowMs() + 900;
    const int nextMute = status_.desktop_muted ? 0 : 1;
    salsa_set_master_volume(static_cast<int>(status_.desktop_volume_pct), nextMute);
    status_.desktop_muted = nextMute;
    if (osd_) {
        osd_->showVolume(
            static_cast<int>(status_.desktop_volume_pct), nextMute != 0,
            cFixedStr(status_.hw_device, sizeof(status_.hw_device)),
            false, theme_.qcolor("accent_primary", "#00e676"), fontPct_, lastTrayRawPt_);
    }
    syncFromEngine(false);
}

void TrayController::toggleMicMute() {
    if (!engineOnline_) return;
    osdSuppressUntilMs_ = nowMs() + 900;
    const int nextMute = status_.desktop_mic_muted ? 0 : 1;
    salsa_set_mic_volume(static_cast<int>(status_.desktop_mic_volume_pct), nextMute);
    status_.desktop_mic_muted = nextMute;
    if (osd_) {
        osd_->showVolume(
            static_cast<int>(status_.desktop_mic_volume_pct), nextMute != 0,
            cFixedStr(status_.hw_capture_device, sizeof(status_.hw_capture_device)),
            true, theme_.qcolor("accent_secondary", "#38bdf8"), fontPct_, lastTrayRawPt_);
    }
    syncFromEngine(false);
}

void TrayController::handleScroll(int delta, const QString& orientation) {
    if (!engineOnline_ || delta == 0) return;
    osdSuppressUntilMs_ = nowMs() + 900;
    const int notches = std::max<int>(1, static_cast<int>(std::lround(std::abs(delta) / 120.0)));
    const int step = (delta > 0 ? 1 : -1) * volStepPct_ * notches;
    const bool shiftHeld = (QApplication::keyboardModifiers() & Qt::ShiftModifier) != 0;
    const bool isHorizontal = orientation.trimmed().toLower().startsWith('h');

    if (isHorizontal || shiftHeld) {
        const int cur = static_cast<int>(status_.desktop_mic_volume_pct);
        const int nxt = std::clamp(cur + step, 0, 150);
        micHoldUntilMs_ = nowMs() + 450;
        lastMicVol_ = nxt;
        lastMicMute_ = (status_.desktop_mic_muted != 0);
        salsa_set_mic_volume(nxt, -1);
        status_.desktop_mic_volume_pct = static_cast<uint32_t>(nxt);
        if (osd_) {
            osd_->showVolume(
                nxt, status_.desktop_mic_muted != 0,
                cFixedStr(status_.hw_capture_device, sizeof(status_.hw_capture_device)),
                true, theme_.qcolor("accent_secondary", "#38bdf8"), fontPct_, lastTrayRawPt_);
        }
    } else {
        const int cur = static_cast<int>(status_.desktop_volume_pct);
        const int nxt = std::clamp(cur + step, 0, 150);
        spkHoldUntilMs_ = nowMs() + 450;
        lastSpkVol_ = nxt;
        lastSpkMute_ = (status_.desktop_muted != 0);
        salsa_set_master_volume(nxt, -1);
        status_.desktop_volume_pct = static_cast<uint32_t>(nxt);
        if (osd_) {
            osd_->showVolume(
                nxt, status_.desktop_muted != 0,
                cFixedStr(status_.hw_device, sizeof(status_.hw_device)),
                false, theme_.qcolor("accent_primary", "#00e676"), fontPct_, lastTrayRawPt_);
        }
    }
    if (popupWin_ && popupWin_->isVisible()) populateQuickMixer();
    syncFromEngine(false);
}

void TrayController::populateDeviceCombos() {
    if (!cmbOutDev_ || !cmbMicDev_) return;
    const QString actPb = cFixedStr(status_.active_playback_id, sizeof(status_.active_playback_id));
    const QString actCap = cFixedStr(status_.active_capture_id, sizeof(status_.active_capture_id));

    QString sig = actPb + QChar('|') + actCap + QChar('|');
    for (uint32_t i = 0; i < std::min<uint32_t>(devices_.num_devices, SALSA_MAX_DEVICES); ++i) {
        sig += cFixedStr(devices_.devices[i].id, sizeof(devices_.devices[i].id)) + QChar(',')
               + cFixedStr(devices_.devices[i].name, sizeof(devices_.devices[i].name)) + QChar(';');
    }
    if (sig == lastDevSig_) return;
    if (cmbOutDev_->view()->isVisible() || cmbMicDev_->view()->isVisible()) return;
    lastDevSig_ = sig;

    cmbOutDev_->clear();
    cmbMicDev_->clear();
    int pbIdx = 0;
    int capIdx = 0;
    int pI = 0;
    int cI = 0;
    for (uint32_t i = 0; i < std::min<uint32_t>(devices_.num_devices, SALSA_MAX_DEVICES); ++i) {
        const auto& d = devices_.devices[i];
        const QString id = cFixedStr(d.id, sizeof(d.id));
        const QString nm = cFixedStr(d.name, sizeof(d.name));
        const QString disp = nm.isEmpty() ? id : nm;
        if (d.is_capture) {
            cmbMicDev_->addItem(disp, id);
            if (id == actCap || (actCap.isEmpty() && d.is_active)) capIdx = cI;
            ++cI;
        } else {
            cmbOutDev_->addItem(disp, id);
            if (id == actPb || (actPb.isEmpty() && d.is_active)) pbIdx = pI;
            ++pI;
        }
    }
    if (cmbOutDev_->count() > 0) cmbOutDev_->setCurrentIndex(pbIdx);
    if (cmbMicDev_->count() > 0) cmbMicDev_->setCurrentIndex(capIdx);
}

void TrayController::updateRateComboLabels(const QString& pbId) {
    if (!cmbRate_ || cmbRate_->view()->isVisible()) return;
    if (pbId != lastPbForRates_) {
        lastPbForRates_ = pbId;
        supportedPbRates_ = probeAlsaPlaybackRates(pbId);
    }
    for (int i = 0; i < cmbRate_->count(); ++i) {
        const int rVal = qvarInt(cmbRate_->itemData(i), 0);
        const QString base = QStringLiteral("%1 kHz").arg(rVal / 1000.0, 0, 'g', 4);
        const bool na = !supportedPbRates_.isEmpty() && !supportedPbRates_.contains(rVal);
        const QString lbl = na ? QStringLiteral("%1 (N/A)").arg(base) : base;
        if (cmbRate_->itemText(i) != lbl) cmbRate_->setItemText(i, lbl);
    }
}

void TrayController::populateQuickMixer() {
    if (!popupWin_) return;
    syncingUi_ = true;

    const bool perDevEq = guiSettings_.contains(QStringLiteral("eq_per_device_enabled"))
                              ? guiSettings_.value(QStringLiteral("eq_per_device_enabled")).toBool(false)
                              : guiSettings_.value(QStringLiteral("eq_per_device")).toBool(false);
    if (btnPerDevEq_) btnPerDevEq_->setChecked(perDevEq);
    if (btnVolStep_) btnVolStep_->setText(QStringLiteral("Step ±%1%").arg(volStepPct_));

    if (!engineOnline_) {
        lblStatusPill_->setText(QStringLiteral("STOPPED"));
        lblStatusPill_->setStyleSheet(
            "background-color: rgba(245,158,11,36); color: #F59E0B; border: 1px solid #F59E0B;"
            "border-radius: 8px; padding: 2px 7px; font-weight: 700;");
        lblHwBadge_->setText(QStringLiteral("Engine Offline"));
        btPill_->hide();
        peakBar_->setPeaks(0.0f, 0.0f, theme_.qcolor("accent_primary", "#00e676"));
        for (const auto& ar : appRows_) ar.row->deleteLater();
        appRows_.clear();
        if (lblNoApps_) lblNoApps_->show();
        if (lblAppCount_) lblAppCount_->setText(QStringLiteral("0 active"));
        if (btnTabApps_) btnTabApps_->setText(QStringLiteral("Applications"));
        syncingUi_ = false;
        return;
    }

    lblStatusPill_->setText(status_.desktop_muted ? QStringLiteral("MUTED") : QStringLiteral("ACTIVE"));
    lblStatusPill_->setStyleSheet(QString());

    const QString fifo = status_.realtime_priority > 0 ? QStringLiteral("FIFO") : QStringLiteral("Norm");
    QString hwTag;
    if (!status_.bt_active) {
        if (status_.hw_analog_info.dac_unity_active) {
            hwTag = QStringLiteral(" · DAC");
        } else if (status_.hw_analog_cfg.analog_boost_db > 0.05f) {
            hwTag = QStringLiteral(" · +%1dB").arg(status_.hw_analog_cfg.analog_boost_db, 0, 'f', 1);
        }
    }
    const uint32_t capR = status_.hw_analog_info.reserved;
    QString capTag;
    if (capR >= 8000 && capR <= 384000 && capR != status_.hw_sample_rate) {
        capTag = QStringLiteral(" (Mic %1k)").arg(capR / 1000.0, 0, 'g', 4);
    }
    lblHwBadge_->setText(QStringLiteral("%1 kHz%2 · %3b · %4%5")
                             .arg(status_.hw_sample_rate / 1000.0, 0, 'g', 4)
                             .arg(capTag)
                             .arg(status_.hw_format_bits)
                             .arg(fifo, hwTag));
    const QString archTip = cFixedStr(status_.hw_analog_info.card_arch_name, sizeof(status_.hw_analog_info.card_arch_name));
    const QString diagTip = cFixedStr(status_.hw_analog_info.diag_note, sizeof(status_.hw_analog_info.diag_note));
    if (!archTip.isEmpty() || !diagTip.isEmpty()) {
        lblHwBadge_->setToolTip(QStringLiteral("%1\n%2").arg(archTip, diagTip).trimmed());
    }

    if (status_.bt_active) {
        const QString alias = cFixedStr(status_.bt_alias, sizeof(status_.bt_alias));
        const QString dispAlias = alias.isEmpty() ? QStringLiteral("Bluetooth") : alias;
        const QString codec = cFixedStr(status_.bt_codec, sizeof(status_.bt_codec));
        const QString batS = (btPct_ >= 0) ? QStringLiteral(" · %1%").arg(btPct_) : QString();
        const QString kbpsS = status_.bt_codec_cfg.effective_kbps > 0
                                  ? QStringLiteral("@%1k").arg(status_.bt_codec_cfg.effective_kbps)
                                  : QString();
        lblBtInfo_->setText(QStringLiteral("%1 · %2%3%4 · %5 ms")
                                .arg(dispAlias, codec.isEmpty() ? QStringLiteral("SBC") : codec, kbpsS, batS)
                                .arg(status_.bt_delay_ms));
        btPill_->show();
    } else {
        btPill_->hide();
    }

    populateDeviceCombos();

    const qint64 tnow = nowMs();

    if (!faderSpk_->isDragging() && tnow >= spkHoldUntilMs_) {
        faderSpk_->setLabel(cFixedStr(status_.hw_device, sizeof(status_.hw_device)));
        faderSpk_->setValue(static_cast<int>(status_.desktop_volume_pct), status_.desktop_muted != 0);
    }
    btnSpkMute_->setText(status_.desktop_muted ? QStringLiteral("Unmute") : QStringLiteral("Mute"));
    peakBar_->setPeaks(status_.output_peak_l, status_.output_peak_r, theme_.qcolor("accent_primary", "#00e676"));

    if (!faderMic_->isDragging() && tnow >= micHoldUntilMs_) {
        faderMic_->setLabel(cFixedStr(status_.hw_capture_device, sizeof(status_.hw_capture_device)));
        faderMic_->setValue(static_cast<int>(status_.desktop_mic_volume_pct), status_.desktop_mic_muted != 0);
    }
    btnMicMute_->setText(status_.desktop_mic_muted ? QStringLiteral("Unmute") : QStringLiteral("Mute"));

    // Quick DSP state sync
    if (tnow >= eqHoldUntilMs_) {
        btnEqBypass_->setChecked(status_.eq_cfg.enabled == 0);
        btnLimiter_->setChecked(status_.eq_cfg.limiter_enabled != 0);
        btnCrossfeed_->setChecked(status_.param_eq.crossfeed_enabled != 0);
        if (!cmbQuickEqPreset_->view()->isVisible()) {
            const QString pn = cFixedStr(status_.eq_cfg.preset_name, sizeof(status_.eq_cfg.preset_name));
            int pi = cmbQuickEqPreset_->findData(QStringLiteral("autoeq:") + pn);
            if (pi < 0) pi = cmbQuickEqPreset_->findData(QStringLiteral("builtin:") + pn);
            if (pi >= 0) cmbQuickEqPreset_->setCurrentIndex(pi);
        }
    }

    // BT Codec & ABR
    const uint32_t supMask = status_.bt_codec_cfg.supported_codecs_mask;
    if (!cmbBtCodec_->view()->isVisible()) {
        static const char* kBtNames[6] = {"BT: Auto", "LDAC", "aptX HD", "aptX", "AAC", "SBC"};
        for (int i = 0; i < 6 && i < cmbBtCodec_->count(); ++i) {
            const bool na = (i > 0 && supMask != 0 && !(supMask & (1u << (i - 1))));
            const QString lbl = na ? QStringLiteral("%1 (N/A)").arg(QLatin1String(kBtNames[i]))
                                   : QString::fromLatin1(kBtNames[i]);
            if (cmbBtCodec_->itemText(i) != lbl) cmbBtCodec_->setItemText(i, lbl);
        }
        if (tnow >= btHoldUntilMs_) {
            const int idxBt = cmbBtCodec_->findData(static_cast<int>(status_.bt_codec_cfg.preferred_codec));
            if (idxBt >= 0) cmbBtCodec_->setCurrentIndex(idxBt);
            btnBtAbr_->setChecked(status_.bt_codec_cfg.wireless_abr_enabled != 0);
        }
    }

    // Playback & Capture sample rate combos
    const QString pbId = cFixedStr(status_.active_playback_id, sizeof(status_.active_playback_id));
    updateRateComboLabels(pbId);
    const int wantRate = static_cast<int>(status_.hw_sample_rate);
    if (tnow >= rateHoldUntilMs_ && !cmbRate_->view()->isVisible()) {
        const int idxR = cmbRate_->findData(wantRate);
        if (idxR >= 0) cmbRate_->setCurrentIndex(idxR);
    }
    if (tnow >= capRateHoldUntilMs_ && !cmbCapRate_->view()->isVisible()) {
        const int curCapSel = qvarInt(cmbCapRate_->currentData(), 0);
        int wantCap = 0;
        if (capR >= 44100 && capR <= 192000 && static_cast<int>(capR) != wantRate) {
            wantCap = static_cast<int>(capR);
        } else if (static_cast<int>(capR) == wantRate && curCapSel == wantRate) {
            wantCap = wantRate;
        }
        const int idxC = cmbCapRate_->findData(wantCap);
        if (idxC >= 0) cmbCapRate_->setCurrentIndex(idxC);
    }

    // Active Application streams
    const uint32_t ns = std::min<uint32_t>(status_.num_sessions, SALSA_MAX_SESSIONS);
    lblAppCount_->setText(QStringLiteral("%1 active").arg(ns));
    btnTabApps_->setText(ns > 0 ? QStringLiteral("Applications (%1)").arg(ns) : QStringLiteral("Applications"));
    lblNoApps_->setVisible(ns == 0);

    bool rebuildApps = (appRows_.size() != ns);
    if (!rebuildApps) {
        for (uint32_t i = 0; i < ns; ++i) {
            if (appRows_[i].sid != status_.sessions[i].session_id) {
                rebuildApps = true;
                break;
            }
        }
    }
    if (rebuildApps) {
        for (const auto& ar : appRows_) ar.row->deleteLater();
        appRows_.clear();
        for (uint32_t i = 0; i < ns; ++i) {
            const auto& s = status_.sessions[i];
            const uint32_t sid = s.session_id;
            const QString rawNm = cFixedStr(s.client_name, sizeof(s.client_name));
            const QString nm = QStringLiteral("%1%2").arg(
                s.is_capture ? QStringLiteral("[MIC] ") : QString(),
                rawNm.isEmpty() ? QStringLiteral("App #%1").arg(s.client_pid) : rawNm);

            auto* rw = new QWidget();
            auto* rlay = new QHBoxLayout(rw);
            rlay->setContentsMargins(0, 0, 0, 0);
            rlay->setSpacing(6);

            auto* iconLbl = new QLabel(rw);
            iconLbl->setFixedSize(22, 22);
            iconLbl->setAlignment(Qt::AlignCenter);
            iconLbl->setPixmap(iconResolver_.pixmapFor(rawNm, s.client_pid, s.is_capture != 0, 20));
            rlay->addWidget(iconLbl);

            auto* fader = new SwipeFader(
                nm,
                theme_.qcolor(s.is_capture ? "accent_mic" : "accent_primary", "#00e676"),
                true,
                rw);
            fader->updateMetrics(fontPct_);
            fader->setStepProvider([this]() { return volStepPct_; });
            fader->setThemeColors(
                theme_.qcolor(s.is_capture ? "accent_mic" : "accent_primary", "#00e676"),
                theme_.qcolor("bg_deep", "#0c0e11"),
                theme_.qcolor("border_control", "#334155"),
                theme_.qcolor("text_bright", "#f8fafc"),
                theme_.qcolor("text_muted", "#94a3b8"));
            fader->setValue(static_cast<int>(s.volume_milli / 10), s.muted != 0);
            connect(fader, &SwipeFader::valueChanged, this, [this, sid](int v) {
                const qint64 now = nowMs();
                for (auto& ar : appRows_) {
                    if (ar.sid != sid) continue;
                    ar.holdUntilMs = now + 450;
                    if (now - ar.lastSentMs >= 35) {
                        ar.lastSentMs = now;
                        salsa_set_session_control(sid, v, -1, nullptr);
                    }
                    break;
                }
            });
            connect(fader, &SwipeFader::valueCommitted, this, [this, sid](int v) {
                const qint64 now = nowMs();
                for (auto& ar : appRows_) {
                    if (ar.sid != sid) continue;
                    ar.holdUntilMs = now + 450;
                    ar.lastSentMs = now;
                    break;
                }
                salsa_set_session_control(sid, v, -1, nullptr);
            });
            rlay->addWidget(fader, 1);

            auto* btnM = new QPushButton(s.muted ? QStringLiteral("Unmute") : QStringLiteral("Mute"), rw);
            btnM->setObjectName(QStringLiteral("MuteBtn"));
            btnM->setFixedSize(56, 28);
            btnM->setCursor(Qt::PointingHandCursor);
            if (s.muted) {
                btnM->setStyleSheet(QStringLiteral("color: #EF4444; border-color: #EF4444; font-weight: 700;"));
            }
            connect(btnM, &QPushButton::clicked, this, [this, sid, btnM]() {
                const bool m = (btnM->text() == QLatin1String("Mute"));
                salsa_set_session_control(sid, -1, m ? 1 : 0, nullptr);
                syncFromEngine(false);
            });
            rlay->addWidget(btnM);
            appsLay_->insertWidget(appsLay_->count() - 1, rw);
            appRows_.push_back({sid, rw, iconLbl, fader, btnM, 0, 0});
        }
    } else {
        for (uint32_t i = 0; i < ns; ++i) {
            const auto& s = status_.sessions[i];
            const QString rawNm = cFixedStr(s.client_name, sizeof(s.client_name));
            const QString nm = QStringLiteral("%1%2").arg(
                s.is_capture ? QStringLiteral("[MIC] ") : QString(),
                rawNm.isEmpty() ? QStringLiteral("App #%1").arg(s.client_pid) : rawNm);
            appRows_[i].iconLbl->setPixmap(iconResolver_.pixmapFor(rawNm, s.client_pid, s.is_capture != 0, 20));
            appRows_[i].fader->setLabel(nm);
            if (!appRows_[i].fader->isDragging() && tnow >= appRows_[i].holdUntilMs) {
                appRows_[i].fader->setValue(static_cast<int>(s.volume_milli / 10), s.muted != 0);
            }
            appRows_[i].btnMute->setText(s.muted ? QStringLiteral("Unmute") : QStringLiteral("Mute"));
            appRows_[i].btnMute->setStyleSheet(
                s.muted ? QStringLiteral("color: #EF4444; border-color: #EF4444; font-weight: 700;") : QString());
        }
    }

    syncingUi_ = false;
    keepBottomAnchored();
}

void TrayController::syncFromEngine(bool showOsdIfExternal) {
    SalsaEngineStatus st{};
    const bool ok = (salsa_get_engine_status(&st) == 0);
    engineOnline_ = ok;
    const qint64 tnow = nowMs();

    if (ok) {
        status_ = st;
        if (tnow - lastDevPollMs_ >= 800 || showOsdIfExternal) {
            lastDevPollMs_ = tnow;
            salsa_list_devices(&devices_);
        }

        if (st.bt_active) {
            const QString alias = cFixedStr(st.bt_alias, sizeof(st.bt_alias));
            if (alias != lastBtAlias_) {
                lastBtAlias_ = alias;
                lastBtPollMs_ = 0;
            }
            if (tnow - lastBtPollMs_ > 10000 || btPct_ < 0) {
                lastBtPollMs_ = tnow;
                const BtBatteryInfo bi = readBluezBatteryPct(alias);
                btPct_ = bi.pct;
                btAddr_ = bi.addr;
            }
        } else {
            btPct_ = -1;
            btAddr_.clear();
            lastBtAlias_.clear();
        }

        const int curSpk = static_cast<int>(st.desktop_volume_pct);
        const bool curSpkM = st.desktop_muted != 0;
        const int curMic = static_cast<int>(st.desktop_mic_volume_pct);
        const bool curMicM = st.desktop_mic_muted != 0;

        if (osdInitDone_ && showOsdIfExternal && tnow >= osdSuppressUntilMs_) {
            if (curSpk != lastSpkVol_ || curSpkM != lastSpkMute_) {
                osd_->showVolume(curSpk, curSpkM, cFixedStr(st.hw_device, sizeof(st.hw_device)),
                                 false, theme_.qcolor("accent_primary", "#00e676"), fontPct_, lastTrayRawPt_);
            } else if (curMic != lastMicVol_ || curMicM != lastMicMute_) {
                osd_->showVolume(curMic, curMicM, cFixedStr(st.hw_capture_device, sizeof(st.hw_capture_device)),
                                 true, theme_.qcolor("accent_secondary", "#38bdf8"), fontPct_, lastTrayRawPt_);
            }
        }
        lastSpkVol_ = curSpk;
        lastSpkMute_ = curSpkM;
        lastMicVol_ = curMic;
        lastMicMute_ = curMicM;
        osdInitDone_ = true;
    }

    QString trayTitle;
    if (ok) {
        const QString volS = status_.desktop_muted ? QStringLiteral("MUTED") : QStringLiteral("%1%").arg(status_.desktop_volume_pct);
        const QString micS = status_.desktop_mic_muted ? QStringLiteral("MUTED") : QStringLiteral("%1%").arg(status_.desktop_mic_volume_pct);
        const QString rateS = QStringLiteral("%1 kHz/%2-bit")
                                  .arg(status_.hw_sample_rate / 1000.0, 0, 'g', 4)
                                  .arg(status_.hw_format_bits);
        QString btS;
        if (status_.bt_active) {
            const QString alias = cFixedStr(status_.bt_alias, sizeof(status_.bt_alias));
            const QString codec = cFixedStr(status_.bt_codec, sizeof(status_.bt_codec));
            const QString batFrag = (btPct_ >= 0) ? QStringLiteral(" · %1%").arg(btPct_) : QString();
            btS = QStringLiteral(" · %1 (%2%3)")
                      .arg(alias.isEmpty() ? QStringLiteral("Bluetooth") : alias,
                           codec.isEmpty() ? QStringLiteral("SBC") : codec,
                           batFrag);
        }
        trayTitle = QStringLiteral("Salsa — Out %1 · Mic %2%3 · %4").arg(volS, micS, btS, rateS);
        if (actStatus_) actStatus_->setText(QStringLiteral("Active · Out %1 · Mic %2%3 · %4").arg(volS, micS, btS, rateS));
    } else {
        trayTitle = QStringLiteral("Salsa — Engine Stopped");
        if (actStatus_) actStatus_->setText(QStringLiteral("Salsa — Engine Stopped"));
    }

    if (actMuteSpk_) actMuteSpk_->setText(ok && status_.desktop_muted ? QStringLiteral("Unmute Master Output") : QStringLiteral("Mute Master Output"));
    if (actMuteMic_) actMuteMic_->setText(ok && status_.desktop_mic_muted ? QStringLiteral("Unmute Mic Input") : QStringLiteral("Mute Mic Input"));
    if (actStartEng_) actStartEng_->setEnabled(!ok);
    if (actStopEng_) actStopEng_->setEnabled(ok);
    if (actRestartEng_) actRestartEng_->setEnabled(true);

    if (hdrIcon_ && !iconThemeDir_.isEmpty()) {
        const QString leaf = !ok ? QStringLiteral("salsa-tray-stopped.png")
                                 : (status_.desktop_muted ? QStringLiteral("salsa-tray-muted.png")
                                                          : (trayGlow_ ? QStringLiteral("salsa-tray-active-glow.png")
                                                                       : QStringLiteral("salsa-tray-active-plain.png")));
        const int sz = scaledPx(24, fontPct_);
        if (leaf != lastHdrLeaf_ || sz != lastHdrIconSz_) {
            const QString iconPath = QDir(iconThemeDir_).filePath(QStringLiteral("hicolor/128x128/apps/%1").arg(leaf));
            const QPixmap pm(iconPath);
            if (!pm.isNull()) {
                lastHdrLeaf_ = leaf;
                lastHdrIconSz_ = sz;
                hdrIcon_->setFixedSize(sz, sz);
                hdrIcon_->setPixmap(pm.scaled(sz, sz, Qt::KeepAspectRatio, Qt::SmoothTransformation));
            }
        }
    }

    if (sniHandle_) {
        const int onInt = ok ? 1 : 0;
        const int mutInt = (ok && status_.desktop_muted) ? 1 : 0;
        const int glowInt = trayGlow_ ? 1 : 0;
        if (trayTitle != lastSniTitle_ || onInt != lastSniOnline_ || mutInt != lastSniMuted_ || glowInt != lastSniGlow_) {
            lastSniTitle_ = trayTitle;
            lastSniOnline_ = onInt;
            lastSniMuted_ = mutInt;
            lastSniGlow_ = glowInt;
            const QString stStr = ok ? QStringLiteral("Active") : QStringLiteral("Passive");
            salsa_sni_update(sniHandle_, ok, mutInt != 0, trayGlow_,
                             trayTitle.toUtf8().constData(), stStr.toUtf8().constData());
        }
    }
    const bool popVis = (popupWin_ && popupWin_->isVisible());
    if (popVis) populateQuickMixer();
    if (pollTimer_) {
        const int wantMs = popVis ? salsaTelemetryIntervalMs(popupWin_, 120) : 1200;
        if (pollTimer_->interval() != wantMs) pollTimer_->setInterval(wantMs);
    }
}

void TrayController::checkSettingsSync() {
    const uint64_t mtime = salsa_gui_settings_mtime_ns();
    if (mtime != 0 && mtime != lastSettingsMtimeNs_) {
        lastSettingsMtimeNs_ = mtime;
        guiSettings_ = loadGuiSettings();
        const int diskPct = normalizeFontScalePct(salsa_load_font_scale());
        volStepPct_ = normalizeVolStepPct(guiSettings_.value(QStringLiteral("volume_step_pct")).toInt(volStepPct_));
        if (btnVolStep_) btnVolStep_->setText(QStringLiteral("Step ±%1%").arg(volStepPct_));

        theme_ = loadThemeForApp(true);
        const QString newTName = guiSettings_.value(QStringLiteral("tray_theme")).toString().trimmed();
        if (!newTName.isEmpty()) themeName_ = newTName;
        rebuildEqPresetCombo();
        const bool fontChanged = (diskPct != fontPct_);
        if (fontChanged) {
            fontPct_ = diskPct;
        }
        applyThemeAndFont(false);
        if (fontChanged && popupWin_ && popupWin_->isVisible()) {
            positionAndShowPopup(lastTrayRawPt_);
        }
    }
}

}  // namespace salsa

#include "lwasapi_tray.moc"

int main(int argc, char** argv) {
    salsa_reset_sigpipe();
    bool selfTest = false;
    bool replace = false;
    for (int i = 1; i < argc; ++i) {
        const QString arg = QString::fromUtf8(argv[i]);
        if (arg == QLatin1String("--self-test")) selfTest = true;
        else if (arg == QLatin1String("--replace")) replace = true;
    }

    if (!salsa::verifyBridgeAbi()) {
        std::fprintf(stderr, "FATAL: Salsa ABI struct size mismatch between C++ and Rust\n");
        return 2;
    }

    if (selfTest) {
        qputenv("QT_QPA_PLATFORM", "offscreen");
        QApplication app(argc, argv);
        const salsa::SalsaStr st(salsa_tray_selftest());
        const QString res = st.toQString();
        if (!res.startsWith(QLatin1String("OK:"))) {
            std::fprintf(stderr, "%s\n", res.toUtf8().constData());
            return 1;
        }
        salsa::TrayController ctrl(false);
        std::printf("%s\n", res.toUtf8().constData());
        return 0;
    }

    // Use XCB/XWayland for Plasma StatusNotifierItem popup anchoring + X11 skip_taskbar hints
    if (!qEnvironmentVariableIsSet("QT_QPA_PLATFORM") && qEnvironmentVariableIsSet("DISPLAY")) {
        qputenv("QT_QPA_PLATFORM", "xcb");
    }

    void* lock = salsa_acquire_tray_lock(replace);
    if (!lock) {
        return 0;
    }

    QApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("lwasapi-tray"));
    app.setDesktopFileName(QStringLiteral("salsa-lwasapi"));
    app.setQuitOnLastWindowClosed(false);

    salsa::TrayController ctrl(true);
    const int rc = app.exec();
    salsa_release_tray_lock(lock);
    return rc;
}

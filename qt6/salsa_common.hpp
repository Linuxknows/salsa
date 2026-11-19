// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#pragma once

#include <QApplication>
#include <QColor>
#include <QDialog>
#include <QHBoxLayout>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QMap>
#include <QPalette>
#include <QPushButton>
#include <QSlider>
#include <QString>
#include <QStringList>
#include <QVariant>
#include <QWidget>

#include <functional>

extern "C" {
#include "salsa_bridge.h"
}

namespace salsa {

inline constexpr const char* BAND_FREQS[SALSA_EQ_BANDS] = {
    "31 Hz", "62 Hz", "125 Hz", "250 Hz", "500 Hz",
    "1 kHz", "2 kHz", "4 kHz", "8 kHz", "16 kHz"
};

inline constexpr const char* BAND_FREQS_SHORT[SALSA_EQ_BANDS] = {
    "31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"
};

inline constexpr float DEFAULT_BAND_FREQS_HZ[SALSA_EQ_BANDS] = {
    31.5f, 62.5f, 125.0f, 250.0f, 500.0f, 1000.0f, 2000.0f, 4000.0f, 8000.0f, 16000.0f
};

inline constexpr const char* CUSTOM_THEME_NAME = "Custom";
inline constexpr int SUPPORTED_VOL_STEPS[4] = {1, 2, 5, 10};

class SalsaStr {
public:
    explicit SalsaStr(char* ptr = nullptr) : ptr_(ptr) {}
    ~SalsaStr() {
        if (ptr_) {
            salsa_str_free(ptr_);
        }
    }
    SalsaStr(const SalsaStr&) = delete;
    SalsaStr& operator=(const SalsaStr&) = delete;
    SalsaStr(SalsaStr&& other) noexcept : ptr_(other.ptr_) { other.ptr_ = nullptr; }
    SalsaStr& operator=(SalsaStr&& other) noexcept {
        if (this != &other) {
            if (ptr_) salsa_str_free(ptr_);
            ptr_ = other.ptr_;
            other.ptr_ = nullptr;
        }
        return *this;
    }

    bool empty() const { return !ptr_ || ptr_[0] == '\0'; }
    QString toQString() const { return ptr_ ? QString::fromUtf8(ptr_) : QString(); }
    QByteArray toUtf8() const { return ptr_ ? QByteArray(ptr_) : QByteArray(); }

private:
    char* ptr_;
};

class SalsaBytes {
public:
    SalsaBytes(uint8_t* ptr = nullptr, size_t len = 0) : ptr_(ptr), len_(len) {}
    ~SalsaBytes() {
        if (ptr_) {
            salsa_bytes_free(ptr_, len_);
        }
    }
    SalsaBytes(const SalsaBytes&) = delete;
    SalsaBytes& operator=(const SalsaBytes&) = delete;

    QByteArray toByteArray() const {
        if (!ptr_ || len_ == 0) return {};
        return QByteArray(reinterpret_cast<const char*>(ptr_), static_cast<qsizetype>(len_));
    }

private:
    uint8_t* ptr_;
    size_t len_;
};

struct SalsaThemeData {
    QString name;
    bool isLight = false;
    QMap<QString, QString> colors;

    QString color(const QString& key, const QString& fallback = QStringLiteral("#ffffff")) const {
        auto it = colors.constFind(key);
        if (it != colors.constEnd() && !it->isEmpty()) return *it;
        return fallback;
    }

    QColor qcolor(const QString& key, const QString& fallback = QStringLiteral("#ffffff")) const {
        QColor c(color(key, fallback));
        return c.isValid() ? c : QColor(fallback);
    }
};

inline QString cFixedStr(const uint8_t* buf, size_t maxLen) {
    if (!buf) return {};
    size_t len = 0;
    while (len < maxLen && buf[len] != 0) ++len;
    return QString::fromUtf8(reinterpret_cast<const char*>(buf), static_cast<qsizetype>(len));
}

inline int qvarInt(const QVariant& v, int fallback = 0) {
    bool ok = false;
    const int val = v.toInt(&ok);
    return ok ? val : fallback;
}

bool verifyBridgeAbi();

QJsonObject parseJsonObj(const QString& json);
QJsonArray parseJsonArr(const QString& json);
QString jsonCompact(const QJsonObject& obj);
QString jsonCompact(const QJsonArray& arr);

SalsaThemeData themeFromJsonObj(const QJsonObject& obj);
QJsonObject themeToJsonObj(const SalsaThemeData& theme);
SalsaThemeData loadThemeForApp(bool forTray);
SalsaThemeData themeByName(const QString& name);
SalsaThemeData normalizeThemeData(const SalsaThemeData& theme);
SalsaThemeData deriveHarmonyThemeData(const SalsaThemeData& theme);
QStringList allThemeNames(bool forTray);

QJsonObject loadGuiSettings();
bool updateGuiSettingsPatch(const QJsonObject& patch);
QJsonObject hostProfile();
QString installRootPath();

int normalizeVolStepPct(int step);
int normalizeFontScalePct(int pct);
int scaledPx(int basePx, int fontPct);
int salsaDetectedOutputHz(const QWidget* w = nullptr);
int salsaTelemetryIntervalMs(const QWidget* w = nullptr, int maxHz = 120);

QPalette buildSalsaPalette(const SalsaThemeData& theme);
QString buildSalsaStylesheet(const SalsaThemeData& theme, bool mini, int fontPct);
void applyGlobalFontScale(QApplication* app, int fontPct);

QColor mixColors(const QColor& a, const QColor& b, double t);
QString validHexColor(const QString& candidate, const QString& fallback);

void spawnDetachedWaylandBinary(const QString& binName, const QStringList& args = {});
void showReportDialog(
    QWidget* parent,
    const QString& title,
    const QString& bodyText,
    const SalsaThemeData& theme,
    int fontPct,
    int width = 820,
    int height = 500);
int handleLegalCliArgs(int argc, char** argv, const char* appName);
void showAboutLicenseDialog(
    QWidget* parent,
    const QString& appName,
    const SalsaThemeData& theme,
    int fontPct);

class StepVolumeSlider : public QSlider {
    Q_OBJECT
public:
    explicit StepVolumeSlider(
        std::function<int()> stepProvider = []() { return 5; },
        Qt::Orientation orient = Qt::Orientation::Horizontal,
        QWidget* parent = nullptr);

protected:
    void wheelEvent(QWheelEvent* event) override;
    void keyPressEvent(QKeyEvent* event) override;

private:
    std::function<int()> stepFn_;
};

class FontScaleBar : public QWidget {
    Q_OBJECT
public:
    explicit FontScaleBar(int initialPct = 100, bool compact = false, QWidget* parent = nullptr);

    int fontScalePct() const { return pct_; }
    void setFontScalePct(int pct);

signals:
    void fontScaleChanged(int pct);

private:
    void stepBy(int delta);
    int pct_ = 100;
    bool compact_ = false;
    QPushButton* btnMinus_ = nullptr;
    QPushButton* btnPct_ = nullptr;
    QPushButton* btnPlus_ = nullptr;
};

}  // namespace salsa

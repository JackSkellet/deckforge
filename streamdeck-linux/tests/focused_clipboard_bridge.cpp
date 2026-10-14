// Test-only bridge: forward Deckforge's input_event frames to an owned KWin.
// The production application continues to use its uinput keyboard.
#include <QCoreApplication>
#include <QDBusInterface>
#include <QDBusConnectionInterface>
#include <QDBusUnixFileDescriptor>
#include <QFile>
#include <QFileInfo>
#include <QSaveFile>
#include <QSocketNotifier>
#include <QTimer>
#include <libei.h>
#include <linux/input.h>
#include <unistd.h>
#include <sys/mman.h>
#include <cstring>
#include <cstdio>

int main(int argc, char **argv) {
    QCoreApplication app(argc, argv);
    if (argc != 2) return 2;
    const QString root = QString::fromLocal8Bit(argv[1]);
    if (!root.startsWith("/tmp/deckforge-focused-") || QFileInfo(root).ownerId() != getuid()) return 2;
    auto bus = QDBusConnection::sessionBus();
    auto pid = bus.interface()->servicePid("org.kde.KWin");
    if (!pid.isValid() || pid.value() != qEnvironmentVariableIntValue("DECKFORGE_PRIVATE_KWIN_PID")) return 3;
    QDBusUnixFileDescriptor fd;
    if (qEnvironmentVariableIsSet("DECKFORGE_PRIVATE_EIS_FD")) {
        bool ok = false;
        const int inherited = qEnvironmentVariableIntValue("DECKFORGE_PRIVATE_EIS_FD", &ok);
        if (!ok || inherited < 3) return 4;
        fd.setFileDescriptor(inherited);
    } else {
        QDBusInterface remote("org.kde.KWin", "/org/kde/KWin/EIS/RemoteDesktop", "org.kde.KWin.EIS.RemoteDesktop", bus);
        auto reply = remote.call("connectToEIS", 1);
        if (reply.type() == QDBusMessage::ErrorMessage) { std::fprintf(stderr, "%s\n", qPrintable(reply.errorMessage())); return 4; }
        fd = qvariant_cast<QDBusUnixFileDescriptor>(reply.arguments().at(0));
    }
    auto connection = ei_new_sender(nullptr);
    ei_configure_name(connection, "Deckforge isolated clipboard test");
    if (ei_setup_backend_fd(connection, dup(fd.fileDescriptor())) != 0) { ei_unref(connection); return 5; }
    ei_device *keyboard = nullptr;
    bool textCapability = false;
    QSocketNotifier notifier(ei_get_fd(connection), QSocketNotifier::Read);
    QObject::connect(&notifier, &QSocketNotifier::activated, [&] {
        ei_dispatch(connection);
        while (auto event = ei_get_event(connection)) {
            auto device = ei_event_get_device(event);
            switch (ei_event_get_type(event)) {
            case EI_EVENT_SEAT_ADDED:
                ei_seat_bind_capabilities(ei_event_get_seat(event), EI_DEVICE_CAP_KEYBOARD, EI_DEVICE_CAP_TEXT, nullptr);
                break;
            case EI_EVENT_DEVICE_RESUMED:
                if (ei_device_has_capability(device, EI_DEVICE_CAP_TEXT)) textCapability = true;
                if (ei_device_has_capability(device, EI_DEVICE_CAP_KEYBOARD) && !keyboard) {
                    keyboard = ei_device_ref(device);
                    if (auto keymap = ei_device_keyboard_get_keymap(device)) {
                        const auto size = ei_keymap_get_size(keymap);
                        if (ei_keymap_get_type(keymap) != EI_KEYMAP_TYPE_XKB || size == 0 || size > 8 * 1024 * 1024) { app.exit(11); break; }
                        void *map = mmap(nullptr, size, PROT_READ, MAP_PRIVATE, ei_keymap_get_fd(keymap), 0);
                        if (map == MAP_FAILED) { app.exit(11); break; }
                        const QByteArray bytes(static_cast<const char *>(map), size);
                        munmap(map, size);
                        QSaveFile output(root + "/keymap.xkb");
                        if (!output.open(QIODevice::WriteOnly) || output.write(bytes) != bytes.size() || !output.commit()) { app.exit(11); break; }
                    }
                    ei_device_start_emulating(keyboard, 0);
                    QFile ready(root + "/bridge-ready");
                    if (!ready.open(QIODevice::WriteOnly) || ready.write("ready") != 5) app.exit(10);
                }
                break;
            case EI_EVENT_KEYBOARD_MODIFIERS: {
                QSaveFile output(root + "/keyboard-state.json");
                if (!output.open(QIODevice::WriteOnly)) { app.exit(12); break; }
                output.write(QByteArray("{\"group\":") + QByteArray::number(ei_event_keyboard_get_xkb_group(event)) +
                    ",\"depressed\":" + QByteArray::number(ei_event_keyboard_get_xkb_mods_depressed(event)) +
                    ",\"locked\":" + QByteArray::number(ei_event_keyboard_get_xkb_mods_locked(event)) + "}");
                if (!output.commit()) app.exit(12);
                break;
            }
            case EI_EVENT_DEVICE_REMOVED:
            case EI_EVENT_DEVICE_PAUSED:
                if (device == keyboard) { ei_device_unref(keyboard); keyboard = nullptr; app.exit(6); }
                break;
            case EI_EVENT_DISCONNECT: app.exit(7); break;
            default: break;
            }
            ei_event_unref(event);
        }
    });
    QFile frames(root + "/keys.bin"); QByteArray pending; quint64 count = 0, lastCount = quint64(-1); bool lastText = false;
    QTimer timer;
    QObject::connect(&timer, &QTimer::timeout, [&] {
        if (!keyboard) return;
        if (!frames.isOpen() && !frames.open(QIODevice::ReadOnly)) return;
        pending += frames.readAll();
        while (pending.size() >= qsizetype(sizeof(input_event))) {
            input_event event; std::memcpy(&event, pending.constData(), sizeof(event)); pending.remove(0, sizeof(event));
            if (event.type == EV_KEY && event.code <= 255 && (event.value == 0 || event.value == 1)) {
                ei_device_keyboard_key(keyboard, event.code, event.value == 1); ++count;
            } else if (event.type == EV_SYN && event.code == SYN_REPORT) {
                ei_device_frame(keyboard, ei_now(connection));
            } else { app.exit(8); return; }
        }
        if (count != lastCount || textCapability != lastText) {
            QSaveFile proof(root + "/bridge.json");
            if (!proof.open(QIODevice::WriteOnly)) { app.exit(10); return; }
            proof.write(QByteArray("{\"key_events\":") + QByteArray::number(count) + ",\"text_capability\":" + (textCapability ? "true" : "false") + "}");
            if (!proof.commit()) { app.exit(10); return; }
            lastCount = count; lastText = textCapability;
        }
    });
    timer.start(2); ei_dispatch(connection);
    QTimer::singleShot(60000, [&] { app.exit(9); });
    const int code = app.exec(); if (keyboard) ei_device_unref(keyboard); ei_unref(connection); return code;
}

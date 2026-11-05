//! Linux UI: Slint window (software rendering) with the same design as the
//! Windows one, tray icon (StatusNotifierItem) and single instance.

use std::cell::RefCell;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc::Receiver;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Model, ModelRc, SharedPixelBuffer, SharedString, Timer, TimerMode, VecModel};

use crate::audio::GameChat;
use crate::balance::Move;
use crate::backend::{Backend, Cmd, Event};
use crate::eq::{self, F_MAX, F_MIN, GAIN_LIMIT, PRESETS, Q_MAX, Q_MIN};
use crate::devices::{self, Band, Status};
use crate::i18n::t;
use crate::layout::{auto_off_chips, level_share, level_text, Frame, Layout, Rect};
use crate::settings::{self, Settings};
use crate::sys;

slint::slint! {
    export struct BandPoint { x: length, y: length, label: string, selected: bool }
    export struct Chip { name: string, active: bool }
    export struct GridLine { x: length, label: string }

    component Toggle inherits Rectangle {
        in property <bool> on;
        in property <bool> enabled: true;
        callback toggled();
        width: 44px;
        height: 24px;
        border-radius: 12px;
        background: !enabled ? #1f1f25 : on ? #e31837 : #2a2a31;
        drop-shadow-blur: on && enabled ? 12px : 0px;
        drop-shadow-color: #e3183799;
        animate background { duration: 150ms; }
        Rectangle {
            width: 18px;
            height: 18px;
            y: 3px;
            x: root.on ? root.width - self.width - 3px : 3px;
            border-radius: 9px;
            background: root.enabled ? #f4f4f5 : #55555d;
            animate x { duration: 160ms; easing: ease-out; }
        }
        TouchArea {
            enabled: root.enabled;
            mouse-cursor: pointer;
            clicked => { root.toggled(); }
        }
    }

    component Card inherits Rectangle {
        in property <string> title;
        background: #141418;
        border-radius: 14px;
        border-width: 1px;
        border-color: #24242b;
        Rectangle { x: 18px; y: 20px; width: 3px; height: 14px; border-radius: 1.5px; background: #e31837; }
        Text {
            x: 30px; y: 17px; height: 20px;
            text: root.title;
            color: #f4f4f5;
            font-size: 11.5px;
            font-weight: 700;
            vertical-alignment: center;
        }
        @children
    }

    export component AppWindow inherits Window {
        title: "Nogenuity";
        icon: root.app-icon;
        // Fixed size (in Slint, width/height on the window make it non-resizable); the height
        // depends on the headset's cards
        width: 760px;
        height: root.win-h;
        background: #0b0b0d;

        in property <image> app-icon;
        // Texts (per system language)
        in property <string> t-subtitle;
        in property <string> t-balance;
        in property <string> t-game;
        in property <string> t-chat;
        in property <string> t-mic-sound;
        in property <string> t-mic-monitoring;
        in property <string> t-mic-monitoring-sub;
        in property <string> t-surround;
        in property <string> t-surround-sub;
        in property <string> t-equalizer;
        in property <string> t-eq-warn;
        in property <string> t-autostart;
        in property <string> t-experimental;
        in property <string> t-headset;
        in property <string> t-sidetone-level;
        in property <string> t-auto-off;
        in property <string> t-voice-prompts;
        in property <string> t-voice-prompts-sub;
        // Layout per headset (layout.rs)
        in property <length> win-h: 724px;
        in property <string> model-title: "NOGENUITY";
        in property <bool> experimental;
        in property <bool> show-balance: true;
        in property <length> bal-x: 20px;
        in property <length> bal-y: 84px;
        in property <length> bal-w: 352px;
        in property <length> bal-h: 216px;
        in property <bool> show-slider: true;
        in property <bool> show-box-levels: true;
        in property <string> game-level;
        in property <string> chat-level;
        in property <bool> show-mic: true;
        in property <length> mic-x: 388px;
        in property <length> mic-y: 84px;
        in property <length> mic-w: 352px;
        in property <length> mic-h: 216px;
        in property <bool> show-mic-status: true;
        in property <bool> show-sidetone: true;
        in property <length> sidetone-y: 120px;
        in property <bool> show-surround: true;
        in property <length> surround-y: 166px;
        in property <bool> show-headset;
        in property <length> hs-x: 20px;
        in property <length> hs-y: 316px;
        in property <length> hs-w: 720px;
        in property <length> hs-h: 216px;
        in property <bool> show-level;
        in property <length> level-y: 56px;
        in property <bool> show-auto-off;
        in property <length> auto-off-y: 102px;
        in property <bool> show-voice;
        in property <length> voice-y: 148px;
        in property <bool> show-eq: true;
        in property <length> eq-y: 316px;
        in property <length> footer-y: 676px;
        // State
        in property <string> conn-text;
        in property <color> conn-color: #8d8d96;
        in property <float> balance;
        in property <string> levels-text;
        in property <string> box-text;
        in property <bool> connected;
        in property <bool> mic-muted;
        in property <string> mic-state;
        in property <string> mic-hint;
        in property <bool> sidetone;
        in property <bool> surround;
        in property <bool> autostart;
        // Mic monitoring volume: 0..1, or negative if unknown
        in property <float> level: -1;
        in property <string> level-text;
        in property <[Chip]> auto-off-chips;
        in property <bool> voice-prompts;
        in property <string> log-text;
        in property <string> band-info;
        in property <bool> band-selected;
        in property <bool> eq-warn;
        in property <string> curve;
        in property <string> area;
        in property <[BandPoint]> points;
        in property <[Chip]> chips;
        in property <[GridLine]> grid;

        callback toggle-sidetone();
        callback toggle-surround();
        callback toggle-autostart();
        callback level-dragged(float);
        callback level-released();
        callback auto-off-clicked(int);
        callback toggle-voice-prompts();
        callback preset-clicked(int);
        callback balance-changed(float);
        callback balance-reset();
        callback band-pressed(int);
        callback band-dragged(int, length, length);
        callback band-reset(int);
        callback band-wheel(int, float);
        callback graph-clicked();
        // Footer link: «Dar acceso» without headset permission, or «Enviar informe» on
        // experimental models (empty = hidden)
        in property <string> footer-link;
        callback footer-link-clicked();
        callback quit-requested();

        // Ctrl+Q quits the app: without a tray there is no other «Salir»
        forward-focus: keys;
        keys := FocusScope {
            width: 0px;
            height: 0px;
            key-pressed(event) => {
                if (event.modifiers.control && (event.text == "q" || event.text == "Q")) {
                    root.quit-requested();
                    return accept;
                }
                return reject;
            }
        }

        // --- Header ---
        Rectangle {
            x: -380px; y: -260px; width: 880px; height: 540px;
            background: @radial-gradient(circle, #e3183738 0%, #e3183700 70%);
        }
        Rectangle {
            x: 20px; y: 20px; width: 44px; height: 44px;
            border-radius: 12px;
            background: #e31837;
            drop-shadow-blur: 18px;
            drop-shadow-color: #e3183780;
            Path {
                width: 44px; height: 44px;
                viewbox-width: 44; viewbox-height: 44;
                commands: "M 12 25 A 10 10 0 0 1 32 25";
                stroke: #ffffff;
                stroke-width: 3px;
            }
            Rectangle { x: 10px; y: 22px; width: 7px; height: 12px; border-radius: 2.5px; background: #ffffff; }
            Rectangle { x: 27px; y: 22px; width: 7px; height: 12px; border-radius: 2.5px; background: #ffffff; }
        }
        // Up to the «Experimental» tag; long names in a smaller font
        Text {
            x: 78px; y: 18px; width: 320px; height: 28px;
            text: root.model-title; color: #f4f4f5; font-size: root.model-title.character-count > 16 ? 18px : 22px; font-weight: 700;
            vertical-alignment: center; overflow: elide;
        }
        Text { x: 79px; y: 44px; height: 18px; text: root.t-subtitle; color: #8d8d96; font-size: 11px; vertical-alignment: center; }
        if root.experimental: Rectangle {
            x: 406px; y: 30px; width: 110px; height: 24px; border-radius: 12px;
            background: #f5a5241f;
            border-width: 1px;
            border-color: #f5a52480;
            Text { text: root.t-experimental; color: #f5a524; font-size: 11px; font-weight: 700; horizontal-alignment: center; vertical-alignment: center; }
        }
        Rectangle {
            x: 530px; y: 28px; width: 210px; height: 28px;
            border-radius: 14px;
            background: root.conn-color.with-alpha(0.10);
            border-width: 1px;
            border-color: root.conn-color.with-alpha(0.35);
            Rectangle {
                x: 14px; y: 10px; width: 8px; height: 8px; border-radius: 4px;
                background: root.conn-color;
                drop-shadow-blur: 8px;
                drop-shadow-color: root.conn-color;
            }
            Text {
                x: 26px; width: parent.width - 32px; height: parent.height;
                text: root.conn-text; color: #f4f4f5; font-size: 11px; font-weight: 700;
                horizontal-alignment: center; vertical-alignment: center;
            }
        }
        Rectangle { x: 20px; y: 72px; width: 720px; height: 1px; background: @linear-gradient(90deg, #e318378c 0%, #e3183700 100%); }

        // --- Game / chat balance ---
        Card {
            x: root.bal-x; y: root.bal-y; width: root.bal-w; height: root.bal-h;
            visible: root.show-balance;
            title: root.t-balance;
            // Slider (with Game and Chat outputs in the system), centered in the card
            Rectangle {
            x: (parent.width - 352px) / 2; width: 352px; height: parent.height;
            visible: root.show-slider;
            Rectangle { x: 62px; y: 91px; width: 228px; height: 6px; border-radius: 3px; background: #2a2a31; }
            Rectangle {
                x: Math.min(176px + root.balance / 100 * 114px, 176px);
                y: 91px; height: 6px;
                width: Math.abs(root.balance / 100 * 114px);
                border-radius: 3px;
                background: root.balance < 0 ? #ff3b4f : #c9ced6;
            }
            Rectangle { x: 176px; y: 85px; width: 1px; height: 18px; background: #55555d; }
            Rectangle { x: 38px; y: 90px; width: 8px; height: 8px; border-radius: 4px; background: #ff3b4f; }
            Rectangle { x: 306px; y: 90px; width: 8px; height: 8px; border-radius: 4px; background: #c9ced6; }
            Rectangle {
                x: 176px + root.balance / 100 * 114px - 10px; y: 84px;
                width: 20px; height: 20px; border-radius: 10px;
                background: #f4f4f5;
                drop-shadow-blur: 14px;
                drop-shadow-color: root.balance < 0 ? #e3183799 : #c9ced666;
                Rectangle {
                    x: 6px; y: 6px; width: 8px; height: 8px; border-radius: 4px;
                    background: root.balance < 0 ? #e31837 : #6b6f78;
                }
            }
            Text { x: 58px; y: 108px; height: 16px; text: root.t-game; color: #8d8d96; font-size: 11px; }
            Text { x: 230px; y: 108px; width: 64px; height: 16px; text: root.t-chat; color: #8d8d96; font-size: 11px; horizontal-alignment: right; }
            TouchArea {
                x: 32px; y: 76px; width: 288px; height: 36px;
                mouse-cursor: ew-resize;
                pointer-event(ev) => {
                    if (ev.kind == PointerEventKind.down && ev.button == PointerEventButton.left) {
                        root.balance-changed((self.x + self.mouse-x - 62px) / 228px * 200 - 100);
                    }
                }
                moved => {
                    if (self.pressed) {
                        root.balance-changed((self.x + self.mouse-x - 62px) / 228px * 200 - 100);
                    }
                }
                double-clicked => { root.balance-reset(); }
            }
            }
            if root.show-slider: Text { x: 26px; y: 160px; height: 20px; text: root.levels-text; color: #f4f4f5; font-size: 13px; vertical-alignment: center; }
            if root.show-box-levels: Text { x: 26px; y: 182px; height: 18px; text: root.box-text; color: #8d8d96; font-size: 11px; vertical-alignment: center; }
            // No slider: the box's levels, large
            if !root.show-slider: Rectangle {
                for side[i] in [
                    { name: root.t-game, level: root.game-level, color: #ff3b4f },
                    { name: root.t-chat, level: root.chat-level, color: #c9ced6 },
                ]: Rectangle {
                    x: i == 0 ? 26px : parent.width / 2 + 8px; y: 78px; width: 160px; height: 40px;
                    Rectangle { x: 10px; y: 12px; width: 8px; height: 8px; border-radius: 4px; background: side.color; }
                    Text { x: 36px; y: 0px; height: 22px; text: side.level; color: #f4f4f5; font-size: 17px; font-weight: 700; vertical-alignment: center; }
                    Text { x: 36px; y: 22px; height: 16px; text: side.name; color: #8d8d96; font-size: 11px; vertical-alignment: center; }
                }
            }
        }

        // --- Microphone and sound ---
        Card {
            x: root.mic-x; y: root.mic-y; width: root.mic-w; height: root.mic-h;
            visible: root.show-mic;
            title: root.t-mic-sound;
            Rectangle {
                visible: root.show-mic-status;
                x: 27px; y: 57px; width: 42px; height: 42px; border-radius: 21px;
                background: #1c1c22;
                border-width: 2px;
                border-color: !root.connected ? #55555d : root.mic-muted ? #ff3b4f : #2fd27a;
                drop-shadow-blur: 16px;
                drop-shadow-color: (!root.connected ? #55555d : root.mic-muted ? #ff3b4f : #2fd27a).with-alpha(0.45);
                Rectangle { x: 16px; y: 8px; width: 10px; height: 15px; border-radius: 5px; border-width: 2px; border-color: #f4f4f5; }
                Path {
                    x: 10px; y: 13px; width: 22px; height: 14px;
                    viewbox-width: 22; viewbox-height: 14;
                    commands: "M 2 2 A 9 9 0 0 0 20 2";
                    stroke: #f4f4f5; stroke-width: 2px;
                }
                Rectangle { x: 20px; y: 27px; width: 2px; height: 4px; background: #f4f4f5; }
                Rectangle { x: 16px; y: 31px; width: 10px; height: 2px; border-radius: 1px; background: #f4f4f5; }
                if root.mic-muted: Path {
                    width: 42px; height: 42px; viewbox-width: 42; viewbox-height: 42;
                    commands: "M 10 32 L 32 10";
                    stroke: #ff3b4f; stroke-width: 2.5px;
                }
            }
            if root.show-mic-status: Text { x: 82px; y: 58px; width: 250px; height: 22px; text: root.mic-state; color: root.mic-muted ? #ff3b4f : #f4f4f5; font-size: 17px; font-weight: 700; vertical-alignment: center; }
            if root.show-mic-status: Text { x: 82px; y: 80px; width: 250px; height: 18px; text: root.mic-hint; color: #8d8d96; font-size: 11px; vertical-alignment: center; }

            // One row per setting: title and description on the left, switch on the right
            if root.show-sidetone: Rectangle {
                y: root.sidetone-y - 4px; height: 46px;
                // No line right below the title
                Rectangle { x: 18px; y: 0px; width: parent.width - 36px; height: 1px; background: #24242b; visible: root.show-mic-status; }
                Text { x: 18px; y: 6px; width: 250px; height: 19px; text: root.t-mic-monitoring; color: #f4f4f5; font-size: 13px; }
                Text { x: 18px; y: 24px; width: 250px; height: 17px; text: root.t-mic-monitoring-sub; color: #8d8d96; font-size: 11px; }
                Toggle { x: parent.width - 62px; y: 12px; on: root.sidetone; enabled: root.connected; toggled => { root.toggle-sidetone(); } }
            }
            if root.show-surround: Rectangle {
                y: root.surround-y - 4px; height: 46px;
                Rectangle { x: 18px; y: 0px; width: parent.width - 36px; height: 1px; background: #24242b; visible: root.show-mic-status || root.show-sidetone; }
                Text { x: 18px; y: 6px; width: 250px; height: 19px; text: root.t-surround; color: #f4f4f5; font-size: 13px; }
                Text { x: 18px; y: 24px; width: 250px; height: 17px; text: root.t-surround-sub; color: #8d8d96; font-size: 11px; }
                Toggle { x: parent.width - 62px; y: 12px; on: root.surround; enabled: root.connected; toggled => { root.toggle-surround(); } }
            }
        }

        // --- Headset: mic monitoring volume, auto power-off and voice prompts ---
        Card {
            x: root.hs-x; y: root.hs-y; width: root.hs-w; height: root.hs-h;
            visible: root.show-headset;
            title: root.t-headset;
            if root.show-level: Rectangle {
                y: root.level-y - 4px; height: 46px;
                Rectangle { x: 18px; y: 0px; width: parent.width - 36px; height: 1px; background: #24242b; visible: root.level-y > 56px; }
                Text { x: 18px; y: 6px; width: parent.width - 96px; height: 19px; text: root.t-sidetone-level; color: #f4f4f5; font-size: 13px; }
                Text {
                    x: parent.width - 78px; y: 6px; width: 60px; height: 19px;
                    text: root.level-text; color: root.connected ? #c9ced6 : #55555d; font-size: 11px;
                    horizontal-alignment: right; vertical-alignment: center;
                }
                bar := Rectangle {
                    x: 26px; y: 32px; width: parent.width - 52px; height: 6px; border-radius: 3px;
                    background: #2a2a31;
                }
                if root.level >= 0: Rectangle {
                    x: bar.x; y: bar.y; width: root.level * bar.width; height: 6px; border-radius: 3px;
                    background: root.connected ? #ff3b4f : #55555d;
                }
                if root.level >= 0: Rectangle {
                    x: bar.x + root.level * bar.width - 8px; y: bar.y - 5px; width: 16px; height: 16px; border-radius: 8px;
                    background: root.connected ? #f4f4f5 : #55555d;
                    drop-shadow-blur: root.connected ? 14px : 0px;
                    drop-shadow-color: #e3183799;
                    Rectangle { x: 5px; y: 5px; width: 6px; height: 6px; border-radius: 3px; background: root.connected ? #e31837 : #2a2a31; }
                }
                TouchArea {
                    x: bar.x - 10px; y: bar.y - 12px; width: bar.width + 20px; height: 30px;
                    enabled: root.connected;
                    mouse-cursor: ew-resize;
                    pointer-event(ev) => {
                        if (ev.kind == PointerEventKind.cancel) {
                            // Interrupted drag: send the current value
                            root.level-released();
                        } else if (ev.button == PointerEventButton.left) {
                            if (ev.kind == PointerEventKind.down) {
                                root.level-dragged((self.mouse-x - 10px) / bar.width);
                            } else if (ev.kind == PointerEventKind.up) {
                                root.level-released();
                            }
                        }
                    }
                    moved => {
                        if (self.pressed) {
                            root.level-dragged((self.mouse-x - 10px) / bar.width);
                        }
                    }
                }
            }
            if root.show-auto-off: Rectangle {
                y: root.auto-off-y - 4px; height: 46px;
                Rectangle { x: 18px; y: 0px; width: parent.width - 36px; height: 1px; background: #24242b; visible: root.auto-off-y > 56px; }
                Text { x: 18px; y: 6px; width: parent.width - 36px; height: 19px; text: root.t-auto-off; color: #f4f4f5; font-size: 13px; }
                // One chip per option, spread across the width
                HorizontalLayout {
                    x: 18px; y: 25px; width: parent.width - 36px; height: 18px;
                    spacing: 6px;
                    for c[i] in root.auto-off-chips: Rectangle {
                        horizontal-stretch: 1;
                        border-radius: 9px;
                        background: c.active ? (root.connected ? #e31837 : #2a2a31) : #1f1f25;
                        border-width: c.active ? 0px : 1px;
                        border-color: #24242b;
                        Text {
                            text: c.name; color: !root.connected ? #55555d : c.active ? #f4f4f5 : #c9ced6;
                            font-size: 10px; font-weight: 700;
                            horizontal-alignment: center; vertical-alignment: center;
                        }
                        TouchArea { enabled: root.connected; mouse-cursor: pointer; clicked => { root.auto-off-clicked(i); } }
                    }
                }
            }
            if root.show-voice: Rectangle {
                y: root.voice-y - 4px; height: 46px;
                Rectangle { x: 18px; y: 0px; width: parent.width - 36px; height: 1px; background: #24242b; visible: root.voice-y > 56px; }
                Text { x: 18px; y: 6px; width: parent.width - 96px; height: 19px; text: root.t-voice-prompts; color: #f4f4f5; font-size: 13px; }
                Text { x: 18px; y: 24px; width: parent.width - 96px; height: 17px; text: root.t-voice-prompts-sub; color: #8d8d96; font-size: 11px; overflow: elide; }
                Toggle { x: parent.width - 62px; y: 12px; on: root.voice-prompts; enabled: root.connected; toggled => { root.toggle-voice-prompts(); } }
            }
        }

        // --- Equalizer ---
        Card {
            x: 20px; y: root.eq-y; width: 720px; height: 346px;
            visible: root.show-eq;
            title: root.t-equalizer;
            Text {
                x: 300px; y: 17px; width: 402px; height: 20px;
                text: root.band-info; color: root.band-selected ? #ff3b4f : #8d8d96; font-size: 11px;
                horizontal-alignment: right; vertical-alignment: center;
            }
            if root.eq-warn: Rectangle {
                x: 170px; y: 15px; width: 260px; height: 24px; border-radius: 12px;
                background: #f5a5241f;
                border-width: 1px;
                border-color: #f5a52480;
                Text { text: root.t-eq-warn; color: #f5a524; font-size: 11px; font-weight: 700; horizontal-alignment: center; vertical-alignment: center; }
                TouchArea { mouse-cursor: pointer; clicked => { root.toggle-surround(); } }
            }

            graph := Rectangle {
                x: 44px; y: 52px; width: 658px; height: 204px;
                // Outside the range the box supports (±9 dB)
                Rectangle { y: 0px; height: 25.5px; background: #101013; }
                Rectangle { y: 178.5px; height: 25.5px; background: #101013; }
                for g in [12, 6, 0, -6, -12]: Rectangle {
                    y: (12 - g) / 24 * 204px; height: 1px;
                    background: g == 0 ? #55555d : #1e1e24;
                    Text {
                        x: -40px; y: -8px; width: 32px; height: 16px;
                        text: g == 0 ? "0" : g > 0 ? "+" + g : g;
                        color: #8d8d96; font-size: 11px; horizontal-alignment: right; vertical-alignment: center;
                    }
                }
                for line in root.grid: Rectangle {
                    x: line.x; width: 1px; height: 204px; background: #1e1e24;
                    Text {
                        x: -20px; y: 208px; width: 40px; height: 16px;
                        text: line.label; color: #8d8d96; font-size: 10px; horizontal-alignment: center;
                    }
                }
                TouchArea { clicked => { root.graph-clicked(); } }
                Path {
                    width: 658px; height: 204px; viewbox-width: 658; viewbox-height: 204;
                    commands: root.area;
                    fill: @linear-gradient(180deg, #e3183773 0%, #e3183705 50%, #e3183773 100%);
                }
                Path { width: 658px; height: 204px; viewbox-width: 658; viewbox-height: 204; commands: root.curve; stroke: #e318371a; stroke-width: 10px; }
                Path { width: 658px; height: 204px; viewbox-width: 658; viewbox-height: 204; commands: root.curve; stroke: #e3183740; stroke-width: 5px; }
                Path { width: 658px; height: 204px; viewbox-width: 658; viewbox-height: 204; commands: root.curve; stroke: #ff3b4f; stroke-width: 2.2px; }
                for p[i] in root.points: Rectangle {
                    x: p.x - 12px; y: p.y - 12px; width: 24px; height: 24px;
                    Rectangle {
                        width: p.selected ? 16px : 13px; height: self.width; border-radius: self.width / 2;
                        background: p.selected ? #ff3b4f : #f4f4f5;
                        border-width: p.selected ? 0px : 2px;
                        border-color: #e31837;
                        drop-shadow-blur: p.selected ? 16px : 0px;
                        drop-shadow-color: #e31837;
                    }
                    Text {
                        y: -12px; height: 14px; text: p.label;
                        color: p.selected ? #ff3b4f : #8d8d96; font-size: 10px; font-weight: 700;
                        horizontal-alignment: center;
                    }
                    TouchArea {
                        mouse-cursor: pointer;
                        pointer-event(ev) => {
                            if (ev.kind == PointerEventKind.down && ev.button == PointerEventButton.left) {
                                root.band-pressed(i);
                            }
                        }
                        moved => {
                            if (self.pressed) {
                                root.band-dragged(i, parent.x + self.mouse-x, parent.y + self.mouse-y);
                            }
                        }
                        double-clicked => { root.band-reset(i); }
                        scroll-event(ev) => {
                            root.band-wheel(i, ev.delta-y / 1px);
                            accept
                        }
                    }
                }
            }

            HorizontalLayout {
                x: 22px; y: 292px; width: 676px; height: 30px;
                // Each chip wants its full text, but can shrink (text is elided with «…»)
                // so the row never overflows the card, whatever the language
                spacing: 6px;
                for c[i] in root.chips: Rectangle {
                    horizontal-stretch: 1;
                    preferred-width: label.preferred-width + 14px;
                    min-width: 24px;
                    border-radius: 15px;
                    background: c.active ? #e31837 : #1f1f25;
                    border-width: c.active ? 0px : 1px;
                    border-color: #24242b;
                    drop-shadow-blur: c.active ? 14px : 0px;
                    drop-shadow-color: #e3183766;
                    label := Text {
                        x: 7px; width: parent.width - 14px;
                        text: c.name; color: c.active ? #f4f4f5 : #c9ced6;
                        font-size: 11px; font-weight: 700;
                        horizontal-alignment: center; vertical-alignment: center;
                        overflow: elide;
                    }
                    TouchArea { mouse-cursor: pointer; clicked => { root.preset-clicked(i); } }
                }
            }
        }

        // --- Status bar ---
        Rectangle {
            x: 20px; y: root.footer-y; width: 720px; height: 36px;
            background: #141418;
            border-radius: 12px;
            border-width: 1px;
            border-color: #24242b;
            Rectangle { x: 14.5px; y: 14.5px; width: 7px; height: 7px; border-radius: 3.5px; background: root.conn-color; }
            if root.footer-link != "": Text {
                x: 384px; width: 110px; height: 36px; text: root.footer-link; color: #ff3b4f; font-size: 11px; font-weight: 700;
                horizontal-alignment: right; vertical-alignment: center;
                TouchArea { mouse-cursor: pointer; clicked => { root.footer-link-clicked(); } }
            }
            Text { x: 30px; width: root.footer-link != "" ? 344px : 460px; height: 36px; text: root.log-text; color: #8d8d96; font-size: 11px; vertical-alignment: center; overflow: elide; }
            Rectangle { x: 506px; y: 9px; width: 1px; height: 18px; background: #24242b; }
            Text { x: 514px; width: 146px; height: 36px; text: root.t-autostart; color: #c9ced6; font-size: 11px; horizontal-alignment: right; vertical-alignment: center; }
            Toggle { x: 666px; y: 6px; on: root.autostart; toggled => { root.toggle-autostart(); } }
        }
    }
}

// --- Graph geometry (local coordinates, 658 x 204) ---
const GW: f32 = 658.0;
const GH: f32 = 204.0;
const G_VIEW: f32 = 12.0;

fn fx(f: f32) -> f32 {
    (f / F_MIN).ln() / (F_MAX / F_MIN).ln() * GW
}

fn xf(x: f32) -> f32 {
    F_MIN * (F_MAX / F_MIN).powf((x / GW).clamp(0.0, 1.0))
}

fn gy(g: f32) -> f32 {
    (G_VIEW - g) / (2.0 * G_VIEW) * GH
}

fn yg(y: f32) -> f32 {
    (G_VIEW - y / GH * 2.0 * G_VIEW).clamp(-G_VIEW, G_VIEW)
}

/// Window footer link.
#[derive(Clone, Copy, PartialEq, Debug)]
enum FooterLink {
    None,
    /// Missing udev rule: «Dar acceso»
    Grant,
    /// Experimental model: «Enviar informe»
    Report,
}

/// Permission comes first: without it the model can't be tested.
fn footer_link(needs_access: bool, experimental: bool) -> FooterLink {
    if needs_access {
        FooterLink::Grant
    } else if experimental {
        FooterLink::Report
    } else {
        FooterLink::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_link_prefers_access() {
        assert_eq!(footer_link(true, true), FooterLink::Grant);
        assert_eq!(footer_link(false, true), FooterLink::Report);
        assert_eq!(footer_link(false, false), FooterLink::None);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Conn {
    Searching,
    Connected(u16),
    Disconnected,
    Hung,
}

struct App {
    window: AppWindow,
    settings: Settings,
    backend: Backend,
    events: Receiver<Event>,
    audio: Option<GameChat>,
    win_levels: (f32, f32),
    /// A write to the outputs failed: see `balance::refresh`.
    balance_unapplied: bool,
    status: Option<Status>,
    conn: Conn,
    /// Connected model, or the last one connected
    model: Option<&'static devices::Model>,
    layout: Layout,
    log: String,
    /// Waiting for the user to accept the pkexec dialog
    granting: bool,
    /// The rule was already installed this session (if there's still no access, say so)
    granted: bool,
    /// Link currently shown in the footer: clicking does exactly that
    footer_link: FooterLink,
    /// At login without a tray it was minimized instead of hidden: if the tray shows up shortly
    /// after (desktop still starting), hide into it
    parked_at: Option<Instant>,
    /// Last close without a tray (minimized): a second close right after means it can't be
    /// minimized
    minimized_at: Option<Instant>,
    sel: Option<usize>,
    /// Mic monitoring volume while dragging the bar (sent on release)
    level_drag: Option<u8>,
    freqs: Vec<f32>,
    autostart: bool,
    save_timer: Timer,
    // Persistent models: updated row by row. If they were replaced whole, Slint
    // would recreate the points and the drag in progress would be lost.
    points: Rc<VecModel<BandPoint>>,
    chips: Rc<VecModel<Chip>>,
    auto_off_chips: Rc<VecModel<Chip>>,
}

/// Updates only the rows that changed.
fn sync_model<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) {
    if model.row_count() != rows.len() {
        model.set_vec(rows);
        return;
    }
    for (i, row) in rows.into_iter().enumerate() {
        if model.row_data(i).as_ref() != Some(&row) {
            model.set_row_data(i, row);
        }
    }
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app(f: impl FnOnce(&mut App)) {
    APP.with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            if let Some(app) = slot.as_mut() {
                f(app);
            }
        }
    });
}

impl App {
    fn update_on_connect(&self) {
        *self.backend.on_connect.lock().unwrap() = self.settings.on_connect();
    }

    fn schedule_save(&self) {
        self.save_timer.start(TimerMode::SingleShot, std::time::Duration::from_millis(600), || {
            with_app(|a| a.settings.save());
        });
    }

    fn drain(&mut self) {
        while let Ok(e) = self.events.try_recv() {
            match e {
                Event::Connected { model, firmware } => {
                    self.conn = Conn::Connected(firmware);
                    self.log = model.name.into();
                    self.model = Some(model);
                    self.layout = Layout::for_caps(&model.caps);
                    if self.settings.set_model(model.name) {
                        self.update_on_connect();
                        self.schedule_save();
                    }
                }
                Event::Disconnected => {
                    self.conn = Conn::Disconnected;
                    self.status = None;
                    self.level_drag = None;
                    if !self.granting {
                        self.log = match (sys::needs_udev_rule(), self.granted) {
                            (true, true) => t().access_still_denied,
                            (true, false) => t().udev_hint,
                            (false, _) => t().connect_headset,
                        }
                        .into();
                    }
                }
                Event::Hung => {
                    self.conn = Conn::Hung;
                    self.log = t().hung.into();
                }
                Event::Status(s) => {
                    self.status = Some(s);
                    if !matches!(self.conn, Conn::Connected(_)) {
                        self.conn = Conn::Connected(0);
                    }
                    if self.settings.remember(&self.layout, &s) {
                        self.update_on_connect();
                        self.schedule_save();
                    }
                    // Powered off mid-drag (the bar is disabled and no release arrives): discard
                    if s.asleep {
                        self.level_drag = None;
                    }
                    if self.audio.is_none() {
                        self.audio = self.model.and_then(|m| m.caps.game_chat_outputs).and_then(GameChat::find);
                    }
                    crate::balance::refresh(&mut self.audio, &mut self.win_levels, &mut self.balance_unapplied, &self.settings);
                }
                Event::Button(b) => {
                    let tr = t();
                    self.log = format!(
                        "{}  ·  7.1 {}  ·  {} {:+.0} dB  ·  {} {:+.0} dB",
                        tr.control_box,
                        if b.surround { "ON" } else { "OFF" },
                        tr.game_lc,
                        b.game_db,
                        tr.chat_lc,
                        b.chat_db
                    );
                }
                Event::Log(s) => self.log = s,
            }
        }
        self.refresh();
    }

    fn set_balance(&mut self, value: i32) {
        let Some(audio) = &self.audio else { return };
        let step = match crate::balance::step(value, self.settings.balance, self.settings.balance_base, audio.settled(), || audio.get()) {
            Move::Stay => return,
            Move::To(step) => step,
            Move::Unreadable => {
                // Dropped, so a drag doesn't keep trying: the next status update looks for them again
                self.audio = None;
                self.log = t().no_outputs.into();
                self.refresh();
                return;
            }
        };
        audio.set(step.levels.0, step.levels.1);
        self.settings.balance = step.value;
        self.settings.balance_base = Some(step.base);
        self.win_levels = step.levels;
        self.schedule_save();
        self.refresh();
    }

    fn set_band(&mut self, i: usize, band: Band) {
        self.settings.eq[i] = band;
        self.settings.preset = None;
        self.backend.band(i, band);
        self.update_on_connect();
        self.schedule_save();
        self.refresh();
    }

    fn apply_preset(&mut self, i: usize) {
        self.settings.eq = PRESETS[i].to_vec();
        self.settings.preset = Some(i);
        self.sel = None;
        self.backend.send(Cmd::Eq(self.settings.eq.clone()));
        self.update_on_connect();
        self.schedule_save();
        self.refresh();
    }

    fn set_surround(&mut self, on: bool) {
        self.settings.surround = Some(on);
        if let Some(s) = &mut self.status {
            s.surround = on;
        }
        self.backend.send(Cmd::Surround(on));
        self.update_on_connect();
        self.schedule_save();
        self.refresh();
    }

    fn set_sidetone(&mut self, on: bool) {
        self.settings.sidetone = Some(on);
        if let Some(s) = &mut self.status {
            s.sidetone = on;
        }
        self.backend.send(Cmd::Sidetone(on));
        self.update_on_connect();
        self.schedule_save();
        self.refresh();
    }

    /// Mic monitoring volume on bar release (not on every move: each value is a command).
    fn set_sidetone_level(&mut self, level: u8) {
        self.settings.sidetone_level = Some(level);
        self.update_on_connect();
        self.schedule_save();
        if let Some(s) = &mut self.status {
            s.sidetone_level = Some(level);
        }
        self.backend.send(Cmd::SidetoneLevel(level));
        self.refresh();
    }

    fn set_auto_off(&mut self, minutes: u8) {
        self.settings.auto_off = Some(minutes);
        self.update_on_connect();
        self.schedule_save();
        if let Some(s) = &mut self.status {
            s.auto_off = Some(minutes);
        }
        self.backend.send(Cmd::AutoOff(minutes));
        self.refresh();
    }

    fn set_voice_prompts(&mut self, on: bool) {
        self.settings.voice_prompts = Some(on);
        self.update_on_connect();
        self.schedule_save();
        if let Some(s) = &mut self.status {
            s.voice_prompts = Some(on);
        }
        self.backend.send(Cmd::VoicePrompts(on));
        self.refresh();
    }

    /// Headset card values: those reported by the headset or, if it can't read them (or it's
    /// off), the last known ones.
    fn auto_off(&self) -> Option<u8> {
        self.status.and_then(|s| s.auto_off).or(self.settings.auto_off)
    }

    fn voice_prompts(&self) -> bool {
        self.status.and_then(|s| s.voice_prompts).or(self.settings.voice_prompts).unwrap_or(false)
    }

    /// Only the volume bar (while dragging there's no need to refresh the whole window).
    fn show_level(&self) {
        let caps = self.model.map(|m| m.caps).unwrap_or_default();
        let level = self.level_drag.or(self.status.and_then(|s| s.sidetone_level)).or(self.settings.sidetone_level);
        self.window.set_level(level_share(level, caps.sidetone_level_max).unwrap_or(-1.0));
        self.window.set_level_text(level_text(level, caps.sidetone_level_max, caps.sidetone_level_zero).into());
    }

    /// Places the cards per headset (same layout as on Windows).
    fn apply_layout(&self) {
        let w = &self.window;
        let l = &self.layout;
        let f = Frame::new(l);
        w.set_win_h(f.height);
        w.set_model_title(self.model.map_or("NOGENUITY".to_string(), |m| m.name.trim_start_matches("HyperX ").to_uppercase()).into());
        w.set_experimental(self.model.is_some_and(|m| !m.tested));
        let place = |r: Option<Rect>| r.map_or((0.0, 0.0, 0.0, 0.0), |(x0, y0, x1, y1)| (x0, y0, x1 - x0, y1 - y0));
        let (x, y, width, height) = place(f.balance);
        w.set_show_balance(f.balance.is_some());
        w.set_bal_x(x);
        w.set_bal_y(y);
        w.set_bal_w(width);
        w.set_bal_h(height);
        w.set_show_slider(l.balance_slider);
        w.set_show_box_levels(l.box_levels);
        let (x, y, width, height) = place(f.mic);
        w.set_show_mic(f.mic.is_some());
        w.set_mic_x(x);
        w.set_mic_y(y);
        w.set_mic_w(width);
        w.set_mic_h(height);
        w.set_show_mic_status(l.mic_status);
        // Without the mic indicator, the rows move up into place
        let top = l.mic_rows_top();
        w.set_show_sidetone(l.sidetone);
        w.set_sidetone_y(top);
        w.set_show_surround(l.surround);
        w.set_surround_y(if l.sidetone { top + 46.0 } else { top });
        w.set_t_surround_sub(if l.eq_needs_surround { t().surround_sub } else { t().surround_sub_plain }.into());
        let (x, y, width, height) = place(f.headset);
        w.set_show_headset(f.headset.is_some());
        w.set_hs_x(x);
        w.set_hs_y(y);
        w.set_hs_w(width);
        w.set_hs_h(height);
        let [level_y, auto_off_y, voice_y] = l.headset_rows();
        w.set_show_level(level_y.is_some());
        w.set_level_y(level_y.unwrap_or(0.0));
        w.set_show_auto_off(auto_off_y.is_some());
        w.set_auto_off_y(auto_off_y.unwrap_or(0.0));
        w.set_show_voice(voice_y.is_some());
        w.set_voice_y(voice_y.unwrap_or(0.0));
        w.set_show_eq(f.eq.is_some());
        w.set_eq_y(f.eq.map_or(0.0, |r| r.1));
        w.set_footer_y(f.footer.1);
    }

    /// No permission for the headset (missing udev rule) and no request in progress.
    fn needs_access(&self) -> bool {
        self.status.is_none() && !self.granting && sys::needs_udev_rule()
    }

    /// «Dar acceso»: pkexec on a separate thread (waits for the password), result in the footer. The
    /// backend retries opening the headset every 2 s, so it connects on its own once permitted.
    fn grant_access(&mut self) {
        self.granting = true;
        self.log = t().granting.into();
        self.refresh();
        std::thread::spawn(|| {
            let result = sys::grant_access();
            let _ = slint::invoke_from_event_loop(move || {
                with_app(|a| {
                    a.granting = false;
                    let text = match result {
                        Ok(()) => {
                            a.granted = true;
                            t().access_granted
                        }
                        Err(sys::GrantError::Cancelled) => t().access_cancelled,
                        Err(sys::GrantError::NoPkexec) => t().access_no_pkexec,
                        Err(sys::GrantError::Failed) => t().access_failed,
                    };
                    // If it already connected while pkexec was returning, don't overwrite its message
                    if a.status.is_none() {
                        a.log = text.into();
                    }
                    a.refresh();
                })
            });
        });
    }

    /// Footer link click: does what the visible link said, if it still makes sense.
    fn footer_link_clicked(&mut self) {
        match self.footer_link {
            FooterLink::Grant if self.needs_access() => self.grant_access(),
            FooterLink::Report if self.model.is_some_and(|m| !m.tested) => self.send_report(),
            _ => {}
        }
    }

    /// Experimental model: saves the report, shows its folder and opens the form.
    fn send_report(&mut self) {
        let Some(model) = self.model else { return };
        let firmware = if let Conn::Connected(fw) = self.conn { fw } else { 0 };
        if let Some(path) = crate::report::save(model, firmware, self.status) {
            sys::open(path.parent().unwrap_or(&path));
            sys::open(std::path::Path::new(crate::report::FORM_URL));
            self.log = t().report_saved.into();
            self.refresh();
        }
    }

    /// Pushes all state to the window properties.
    fn refresh(&mut self) {
        self.footer_link = footer_link(self.needs_access(), self.model.is_some_and(|m| !m.tested));
        self.show();
    }

    fn show(&self) {
        self.apply_layout();
        let w = &self.window;
        let tr = t();
        let (conn_text, conn_color) = match self.conn {
            Conn::Searching => (tr.searching.to_string(), 0x8d8d96),
            Conn::Connected(fw) => {
                let p = crate::layout::pill(fw, self.status, &self.layout);
                let color = if p == crate::layout::Pill::HeadsetOff { 0xf5a524 } else { 0x2fd27a };
                (p.text(tr.connected, tr.charging, tr.headset_off), color)
            }
            Conn::Disconnected => (tr.disconnected.to_string(), 0xff3b4f),
            Conn::Hung => (tr.not_responding.to_string(), 0xf5a524),
        };
        w.set_conn_text(conn_text.into());
        w.set_conn_color(slint::Color::from_rgb_u8((conn_color >> 16) as u8, (conn_color >> 8) as u8, conn_color as u8));

        w.set_balance(self.settings.balance as f32);
        let (wg, wc) = self.win_levels;
        w.set_levels_text(if self.audio.is_some() {
            format!("{} {:.0} %  ·  {} {:.0} %", tr.game_lc, wg * 100.0, tr.chat_lc, wc * 100.0).into()
        } else {
            tr.no_outputs.into()
        });
        let level = |db: Option<f32>| db.map_or("—".to_string(), |db| format!("{db:+.0} dB"));
        w.set_game_level(level(self.status.map(|s| s.game_db)).into());
        w.set_chat_level(level(self.status.map(|s| s.chat_db)).into());
        w.set_box_text(match self.status {
            // Without a slider the levels are already shown large: only say where they come from
            _ if !self.layout.balance_slider => tr.control_box.into(),
            Some(s) => format!("{}  ·  {} {:+.0} dB  ·  {} {:+.0} dB", tr.control_box, tr.game_lc, s.game_db, tr.chat_lc, s.chat_db).into(),
            None => format!("{}  ·  —", tr.control_box).into(),
        });

        // With the headset off (only the receiver) the controls don't respond
        let connected = self.status.is_some_and(|s| !s.asleep);
        let muted = self.status.is_some_and(|s| s.mic_muted);
        w.set_connected(connected);
        w.set_mic_muted(muted);
        let (state, hint) = if !connected {
            ("—", tr.connect_headset)
        } else if muted {
            (tr.muted, if self.layout.box_levels { tr.unmute_hint } else { tr.unmute_hint_headset })
        } else {
            (tr.live, if self.layout.box_levels { tr.mute_hint } else { tr.mute_hint_headset })
        };
        w.set_mic_state(state.into());
        w.set_mic_hint(hint.into());
        let surround = self.status.map_or(self.settings.surround.unwrap_or(false), |s| s.surround);
        w.set_sidetone(self.status.map_or(self.settings.sidetone.unwrap_or(false), |s| s.sidetone));
        w.set_surround(surround);
        w.set_eq_warn(self.layout.eq_needs_surround && !surround);
        w.set_autostart(self.autostart);
        self.show_level();
        let options = self.model.map_or(&[][..], |m| m.caps.auto_off);
        let row_w = Frame::new(&self.layout).headset.map_or(0.0, |(x0, _, x1, _)| x1 - x0 - 36.0);
        let (chips, short) = auto_off_chips(options, row_w, tr.never, tr.never_short, tr.minutes);
        w.set_t_auto_off(if short { format!("{} ({})", tr.auto_off, tr.minutes) } else { tr.auto_off.to_string() }.into());
        let current = self.auto_off();
        let chips: Vec<Chip> = chips.into_iter().map(|(name, m)| Chip { name: name.into(), active: current == Some(m) }).collect();
        sync_model(&self.auto_off_chips, chips);
        w.set_voice_prompts(self.voice_prompts());
        w.set_log_text(self.log.clone().into());
        w.set_footer_link(
            match self.footer_link {
                FooterLink::None => "",
                FooterLink::Grant => tr.grant_access,
                FooterLink::Report => tr.send_report,
            }
            .into(),
        );

        w.set_band_selected(self.sel.is_some());
        w.set_band_info(match self.sel {
            Some(i) => {
                let (f, g, q, k) = self.settings.eq[i];
                let hz = if f >= 1000.0 { format!("{:.1} kHz", f / 1000.0) } else { format!("{f:.0} Hz") };
                format!("{} {}  ·  {hz}  ·  {g:+.1} dB  ·  Q {q:.2}{}", tr.band, i + 1, if k == 1 { "  ·  SHELF" } else { "" }).into()
            }
            None => tr.eq_help.into(),
        });

        // Curve and fill
        let resp = eq::response_db(&self.settings.eq, &self.freqs);
        let pts: Vec<(f32, f32)> = self.freqs.iter().zip(&resp).map(|(&f, &db)| (fx(f), gy(db.clamp(-G_VIEW, G_VIEW)))).collect();
        let mut curve = format!("M {:.1} {:.1}", pts[0].0, pts[0].1);
        for (x, y) in &pts[1..] {
            curve += &format!(" L {x:.1} {y:.1}");
        }
        let zero = gy(0.0);
        let area = format!("M {:.1} {zero:.1} L{} L {:.1} {zero:.1} Z", pts[0].0, &curve[1..], pts[pts.len() - 1].0);
        w.set_curve(curve.into());
        w.set_area(area.into());

        let points: Vec<BandPoint> = self
            .settings
            .eq
            .iter()
            .enumerate()
            .map(|(i, &(f, g, _, _))| BandPoint {
                x: fx(f),
                y: gy(g),
                label: SharedString::from((i + 1).to_string()),
                selected: self.sel == Some(i),
            })
            .collect();
        sync_model(&self.points, points);
        let chips: Vec<Chip> = tr
            .presets
            .iter()
            .enumerate()
            .map(|(i, name)| Chip {
                name: name.to_uppercase().into(),
                active: self.settings.preset == Some(i),
            })
            .collect();
        sync_model(&self.chips, chips);
    }
}

fn app_icon() -> slint::Image {
    let size = 64;
    let bgra = crate::icon::pixels(size);
    let mut buf = SharedPixelBuffer::<slint::Rgba8Pixel>::new(size as u32, size as u32);
    for (dst, src) in buf.make_mut_slice().iter_mut().zip(bgra.chunks_exact(4)) {
        *dst = slint::Rgba8Pixel { r: src[2], g: src[1], b: src[0], a: src[3] };
    }
    slint::Image::from_rgba8(buf)
}

// --- Tray icon (StatusNotifierItem: KDE, and GNOME with the AppIndicator extension) ---
struct Tray;

fn show_window() {
    let _ = slint::invoke_from_event_loop(|| {
        with_app(|a| {
            let _ = a.window.show();
            a.window.window().set_minimized(false);
            a.parked_at = None;
            a.minimized_at = None;
            a.refresh();
        })
    });
}

/// There is a notification area (StatusNotifierWatcher) to host the icon. Without it (GNOME without the
/// AppIndicator extension) the app isn't hidden: it's minimized, so it stays in the dock.
static TRAY_ONLINE: AtomicBool = AtomicBool::new(false);

fn tray_online() -> bool {
    TRAY_ONLINE.load(Ordering::Relaxed)
}

fn quit() {
    let _ = slint::invoke_from_event_loop(|| {
        with_app(|a| a.settings.save());
        let _ = slint::quit_event_loop();
    });
}

/// Close without a tray: minimize (the app keeps running to apply settings on plug-in). If it's
/// closed again right away, the window manager doesn't minimize (tiling, WSLg): hide it,
/// and opening the app again shows it. Returns true if it must be hidden.
fn close_without_tray(a: &mut App) -> bool {
    let retried = a.minimized_at.is_some_and(|t| t.elapsed() < Duration::from_secs(2));
    if retried {
        a.minimized_at = None;
    } else {
        a.window.window().set_minimized(true);
        a.minimized_at = Some(Instant::now());
        // Seen when reopening it: explains why it didn't close
        a.log = t().no_tray.into();
        a.refresh();
    }
    retried
}

/// The tray has arrived: if the window was minimized at login because it wasn't there yet, now
/// hide into it, as on a normal start.
fn tray_came_online() {
    let _ = slint::invoke_from_event_loop(|| {
        with_app(|a| {
            if a.parked_at.take().is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
                let _ = a.window.hide();
            }
        })
    });
}

/// The tray is gone: a window hidden in it would become unreachable, so it's
/// shown minimized in the dock.
fn tray_went_offline() {
    let _ = slint::invoke_from_event_loop(|| {
        with_app(|a| {
            if !a.window.window().is_visible() {
                let _ = a.window.show();
                a.window.window().set_minimized(true);
                a.log = t().no_tray.into();
                a.refresh();
            }
        })
    });
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "nogenuity".into()
    }
    fn watcher_online(&self) {
        TRAY_ONLINE.store(true, Ordering::Relaxed);
        tray_came_online();
    }
    /// No tray (or it's gone): the service keeps waiting in case it returns.
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        TRAY_ONLINE.store(false, Ordering::Relaxed);
        tray_went_offline();
        true
    }
    fn title(&self) -> String {
        "Nogenuity".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let size = 32;
        let bgra = crate::icon::pixels(size);
        // ARGB32 in network byte order (big-endian)
        let data = bgra.chunks_exact(4).flat_map(|p| [p[3], p[2], p[1], p[0]]).collect();
        vec![ksni::Icon { width: size as i32, height: size as i32, data }]
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        show_window();
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem { label: t().open.into(), activate: Box::new(|_| show_window()), ..Default::default() }.into(),
            ksni::MenuItem::Separator,
            StandardItem { label: t().exit.into(), activate: Box::new(|_| quit()), ..Default::default() }.into(),
        ]
    }
}

/// Single instance (flock). If another is running, ask it over a socket to show its window.
pub fn claim_single_instance() -> bool {
    static LOCK: OnceLock<std::fs::File> = OnceLock::new();
    let Some(dir) = sys::data_dir() else { return true };
    let _ = std::fs::create_dir_all(&dir);
    let Ok(file) = OpenOptions::new().create(true).write(true).open(dir.join("app.lock")) else { return true };
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        if let Ok(mut s) = UnixStream::connect(sys::runtime_socket()) {
            let _ = s.write_all(b"show");
        }
        return false;
    }
    let _ = LOCK.set(file);
    let socket = sys::runtime_socket();
    let _ = std::fs::remove_file(&socket);
    if let Ok(listener) = UnixListener::bind(&socket) {
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut buf = [0u8; 8];
                if stream.read(&mut buf).is_ok_and(|n| n > 0) {
                    show_window();
                }
            }
        });
    }
    true
}

pub fn run(start_hidden: bool) {
    let window = AppWindow::new().expect("could not create the window");
    // Same id as the .desktop (StartupWMClass): the dock groups the window with its icon. It goes
    // here because Slint refuses it until creating a window has set up its platform, and it has to
    // be set before the window is shown.
    if let Err(e) = slint::set_xdg_app_id("nogenuity") {
        crate::hid::log(&format!("app id: {e}"));
    }
    let tr = t();
    window.set_app_icon(app_icon());
    window.set_t_subtitle(tr.subtitle.into());
    window.set_t_balance(tr.balance.into());
    window.set_t_game(tr.game.into());
    window.set_t_chat(tr.chat.into());
    window.set_t_mic_sound(tr.mic_sound.into());
    window.set_t_mic_monitoring(tr.mic_monitoring.into());
    window.set_t_mic_monitoring_sub(tr.mic_monitoring_sub.into());
    window.set_t_surround(tr.surround.into());
    window.set_t_surround_sub(tr.surround_sub.into());
    window.set_t_equalizer(tr.equalizer.into());
    window.set_t_eq_warn(tr.eq_needs_surround.into());
    window.set_t_autostart(tr.start_with_windows.into());
    window.set_t_experimental(tr.experimental.into());
    window.set_t_headset(tr.headset_card.into());
    window.set_t_sidetone_level(tr.sidetone_level.into());
    window.set_t_voice_prompts(tr.voice_prompts.into());
    window.set_t_voice_prompts_sub(tr.voice_prompts_sub.into());
    let grid: Vec<GridLine> = [(50.0, "50"), (100.0, "100"), (200.0, "200"), (500.0, "500"), (1000.0, "1k"), (2000.0, "2k"), (5000.0, "5k"), (10000.0, "10k"), (20000.0, "20k")]
        .iter()
        .map(|&(f, l)| GridLine { x: fx(f), label: l.into() })
        .collect();
    window.set_grid(ModelRc::new(VecModel::from(grid)));

    let settings = Settings::load();
    let on_connect = settings.on_connect();
    let notify: crate::backend::Notify = Arc::new(|| {
        let _ = slint::invoke_from_event_loop(|| with_app(|a| a.drain()));
    });
    let (backend, events) = Backend::start(notify, on_connect);
    let model = settings.model.as_deref().and_then(devices::find);
    let layout = model.map_or_else(Layout::before_connect, |m| Layout::for_caps(&m.caps));
    let mut audio = model.and_then(|m| m.caps.game_chat_outputs).and_then(GameChat::find);
    let mut win_levels = (0.0, 0.0);
    crate::balance::refresh(&mut audio, &mut win_levels, &mut false, &settings);
    let n = 220;
    let freqs = (0..n).map(|i| F_MIN * (F_MAX / F_MIN).powf(i as f32 / (n - 1) as f32)).collect();

    // Interaction
    window.on_toggle_sidetone(|| with_app(|a| {
        let on = !a.status.is_some_and(|s| s.sidetone);
        a.set_sidetone(on);
    }));
    window.on_toggle_surround(|| with_app(|a| {
        let on = !a.status.map_or(a.settings.surround.unwrap_or(false), |s| s.surround);
        a.set_surround(on);
    }));
    window.on_toggle_autostart(|| with_app(|a| {
        settings::set_autostart(!a.autostart);
        a.autostart = settings::autostart_enabled();
        a.refresh();
    }));
    window.on_level_dragged(|share| with_app(|a| {
        let max = a.model.map_or(0, |m| m.caps.sidetone_level_max);
        a.level_drag = Some((share.clamp(0.0, 1.0) * max as f32).round() as u8);
        a.show_level();
    }));
    window.on_level_released(|| with_app(|a| {
        if let Some(level) = a.level_drag.take() {
            a.set_sidetone_level(level);
        }
    }));
    window.on_auto_off_clicked(|i| with_app(|a| {
        if let Some(&minutes) = a.model.and_then(|m| m.caps.auto_off.get(i as usize)) {
            a.set_auto_off(minutes);
        }
    }));
    window.on_toggle_voice_prompts(|| with_app(|a| {
        let on = !a.voice_prompts();
        a.set_voice_prompts(on);
    }));
    window.on_preset_clicked(|i| with_app(|a| a.apply_preset(i as usize)));
    window.on_balance_changed(|v| with_app(|a| a.set_balance(v.round() as i32)));
    window.on_balance_reset(|| with_app(|a| a.set_balance(0)));
    window.on_footer_link_clicked(|| with_app(|a| a.footer_link_clicked()));
    window.on_graph_clicked(|| with_app(|a| {
        a.sel = None;
        a.refresh();
    }));
    window.on_band_pressed(|i| with_app(|a| {
        a.sel = Some(i as usize);
        a.refresh();
    }));
    window.on_band_dragged(|i, x, y| with_app(|a| {
        let i = i as usize;
        let (_, _, q, kind) = a.settings.eq[i];
        let g = ((yg(y) * 2.0).round() / 2.0).clamp(-GAIN_LIMIT, GAIN_LIMIT);
        a.set_band(i, (xf(x).round(), g, q, kind));
    }));
    window.on_band_reset(|i| with_app(|a| {
        let i = i as usize;
        let (f, _, q, k) = a.settings.eq[i];
        a.set_band(i, (f, 0.0, q, k));
    }));
    window.on_band_wheel(|i, dy| with_app(|a| {
        let i = i as usize;
        let (f, g, q, k) = a.settings.eq[i];
        let factor = if dy > 0.0 { 1.15 } else { 1.0 / 1.15 };
        a.sel = Some(i);
        a.set_band(i, (f, g, ((q * factor * 100.0).round() / 100.0).clamp(Q_MIN, Q_MAX), k));
    }));
    // Closing the window hides it in the tray; without a tray it's minimized. The app keeps running to
    // apply settings when the headset is plugged in.
    window.window().on_close_requested(|| {
        with_app(|a| a.settings.save());
        let mut hide = tray_online();
        if !hide {
            with_app(|a| hide = close_without_tray(a));
        }
        if hide { slint::CloseRequestResponse::HideWindow } else { slint::CloseRequestResponse::KeepWindowShown }
    });
    window.on_quit_requested(quit);

    let mut app = App {
        window: window.clone_strong(),
        settings,
        backend,
        events,
        audio,
        win_levels,
        balance_unapplied: false,
        status: None,
        conn: Conn::Searching,
        model,
        layout,
        log: t().looking.into(),
        granting: false,
        granted: false,
        footer_link: FooterLink::None,
        parked_at: None,
        minimized_at: None,
        sel: None,
        level_drag: None,
        freqs,
        autostart: settings::autostart_enabled(),
        save_timer: Timer::default(),
        points: Rc::new(VecModel::default()),
        chips: Rc::new(VecModel::default()),
        auto_off_chips: Rc::new(VecModel::default()),
    };
    window.set_points(ModelRc::from(app.points.clone()));
    window.set_chips(ModelRc::from(app.chips.clone()));
    window.set_auto_off_chips(ModelRc::from(app.auto_off_chips.clone()));
    app.refresh();
    APP.with(|cell| *cell.borrow_mut() = Some(app));

    {
        use ksni::blocking::TrayMethods;
        // With assume_sni_available, a missing tray isn't an error: it reaches watcher_offline and the
        // icon appears if the tray shows up later (extension enabled, panel restarted)
        TRAY_ONLINE.store(true, Ordering::Relaxed);
        match Tray.assume_sni_available(true).spawn() {
            Ok(handle) => std::mem::forget(handle),
            Err(_) => TRAY_ONLINE.store(false, Ordering::Relaxed),
        }
    }

    // At login (--tray) hide in the tray; without a tray, minimized in the dock
    if !start_hidden || !tray_online() {
        let _ = window.show();
    }
    if start_hidden && !tray_online() {
        // Minimize now (as early as possible, so it's barely visible) and again shortly after, because
        // a freshly created window doesn't always reach the window manager
        window.window().set_minimized(true);
        with_app(|a| a.parked_at = Some(Instant::now()));
        let weak = window.as_weak();
        Timer::single_shot(Duration::from_millis(300), move || {
            if let Some(w) = weak.upgrade() {
                w.window().set_minimized(true);
            }
        });
    }
    if !tray_online() {
        with_app(|a| {
            a.log = t().no_tray.into();
            a.refresh();
        });
    }
    let _ = slint::run_event_loop_until_quit();
}

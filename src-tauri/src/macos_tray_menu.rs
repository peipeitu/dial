//! Quota views embedded in the existing macOS NSMenu, above Open/Quit.
use std::{cell::RefCell, ptr::NonNull};

use block2::RcBlock;
use chrono::{DateTime, Utc};
use objc2::{rc::Retained, MainThreadOnly};
use objc2_app_kit::{
    NSBox, NSBoxType, NSColor, NSEventTrackingRunLoopMode, NSFont, NSMenuItem, NSTextAlignment,
    NSTextField, NSTitlePosition, NSView,
};
use objc2_foundation::{
    MainThreadMarker, NSPoint, NSRect, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString, NSTimer,
};

use crate::{
    provider_label, tray_status_payload, RateLimitWindow, TrayDisplayState, TrayLanguage, TRAY_ID,
};

struct Countdown {
    label: Retained<NSTextField>,
    resets_at: Option<String>,
    language: TrayLanguage,
}

#[derive(Default)]
struct MenuContent {
    item: Option<Retained<NSMenuItem>>,
    pending: Option<TrayDisplayState>,
    countdowns: Vec<Countdown>,
}

thread_local! {
    // AppKit objects never leave the main thread.
    static CONTENT: RefCell<MenuContent> = RefCell::new(MenuContent::default());
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn label(mtm: MainThreadMarker, text: &str, frame: NSRect, bold: bool) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(frame);
    let font = if bold {
        NSFont::boldSystemFontOfSize(12.0)
    } else {
        NSFont::systemFontOfSize(11.0)
    };
    label.setFont(Some(&font));
    label.setTextColor(Some(&NSColor::labelColor()));
    label
}

fn bar(mtm: MainThreadMarker, frame: NSRect, color: &NSColor) -> Retained<NSBox> {
    let view = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
    view.setBoxType(NSBoxType::Custom);
    view.setTitlePosition(NSTitlePosition::NoTitle);
    view.setBorderWidth(0.0);
    view.setCornerRadius(2.0);
    view.setFillColor(color);
    view
}

fn period_label(minutes: u64, language: TrayLanguage) -> String {
    let zh = language == TrayLanguage::Zh;
    if minutes == 10080 {
        return if zh { "每周" } else { "Weekly" }.into();
    }
    for (size, cn, en) in [
        (10080, "周", "weeks"),
        (1440, "天", "days"),
        (60, "小时", "hours"),
    ] {
        if minutes >= size && minutes.is_multiple_of(size) {
            return format!("{} {}", minutes / size, if zh { cn } else { en });
        }
    }
    format!("{} {}", minutes, if zh { "分钟" } else { "minutes" })
}

fn countdown_text(value: Option<&str>, language: TrayLanguage, now: i64) -> String {
    let zh = language == TrayLanguage::Zh;
    let Some(reset) = value.and_then(|v| DateTime::parse_from_rfc3339(v).ok()) else {
        return if zh {
            "更新时间未知"
        } else {
            "Reset time unavailable"
        }
        .into();
    };
    let seconds = reset.timestamp().saturating_sub(now).max(0);
    if seconds == 0 {
        return if zh { "即将更新" } else { "Updating soon" }.into();
    }
    let (days, hours, minutes, secs) = (
        seconds / 86400,
        seconds % 86400 / 3600,
        seconds % 3600 / 60,
        seconds % 60,
    );
    let duration = if zh {
        if days > 0 {
            format!("{days}天 {hours}小时")
        } else if hours > 0 {
            format!("{hours}小时 {minutes}分")
        } else if minutes > 0 {
            format!("{minutes}分 {secs}秒")
        } else {
            format!("{secs}秒")
        }
    } else if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {secs}s")
    } else {
        format!("{secs}s")
    };
    if zh {
        format!("更新 {duration}")
    } else {
        format!("Updates in {duration}")
    }
}

// Use the same window-average projection as the dashboard's meter colors.
fn meter_color(limit: &RateLimitWindow, now: i64) -> Retained<NSColor> {
    let reset = limit
        .resets_at
        .as_deref()
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok());
    let Some(reset) = reset else {
        return NSColor::secondaryLabelColor();
    };
    let duration = limit.window_minutes as f64 * 60.0;
    let elapsed = (now as f64 - (reset.timestamp() as f64 - duration)).clamp(0.0, duration);
    if reset.timestamp() <= now {
        return NSColor::secondaryLabelColor();
    }
    let projected = if limit.used_percent == 0.0 {
        100.0
    } else if elapsed == 0.0 {
        return NSColor::secondaryLabelColor();
    } else {
        100.0 - limit.used_percent.clamp(0.0, 100.0) / elapsed * duration
    };
    if projected > 0.0 {
        NSColor::systemGreenColor()
    } else if projected >= -20.0 {
        NSColor::systemYellowColor()
    } else if projected >= -50.0 {
        NSColor::systemOrangeColor()
    } else {
        NSColor::systemRedColor()
    }
}

fn build_view(
    display: &TrayDisplayState,
    mtm: MainThreadMarker,
) -> (Retained<NSView>, Vec<Countdown>) {
    const WIDTH: f64 = 240.0;
    let mut windows = display.windows.iter().collect::<Vec<_>>();
    windows.sort_by_key(|window| window.window_minutes);
    let height = 26.0
        + if windows.is_empty() {
            24.0
        } else {
            windows.len() as f64 * 58.0
        };
    let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, WIDTH, height));
    let provider = label(
        mtm,
        provider_label(&display.active_provider),
        rect(12.0, height - 22.0, WIDTH - 24.0, 17.0),
        false,
    );
    provider.setTextColor(Some(&NSColor::secondaryLabelColor()));
    view.addSubview(&provider);
    let mut countdowns = Vec::new();
    let now = Utc::now().timestamp();
    if windows.is_empty() {
        let status = label(
            mtm,
            &tray_status_payload(display).status_label,
            rect(12.0, 5.0, WIDTH - 24.0, 17.0),
            false,
        );
        status.setTextColor(Some(&NSColor::secondaryLabelColor()));
        view.addSubview(&status);
    }
    for (index, window) in windows.iter().enumerate() {
        let bottom = height - 26.0 - (index + 1) as f64 * 58.0;
        view.addSubview(&label(
            mtm,
            &period_label(window.window_minutes, display.language),
            rect(12.0, bottom + 36.0, 88.0, 18.0),
            true,
        ));
        let percent = window.remaining_percent.clamp(0.0, 100.0);
        let suffix = if display.language == TrayLanguage::Zh {
            "剩余"
        } else {
            "remaining"
        };
        let value = label(
            mtm,
            &format!("{percent:.0}% {suffix}"),
            rect(100.0, bottom + 36.0, WIDTH - 112.0, 18.0),
            true,
        );
        value.setAlignment(NSTextAlignment::Right);
        view.addSubview(&value);
        let count = match window.window_minutes {
            300 => 5,
            10080 => 7,
            _ => 10,
        };
        let width = (WIDTH - 24.0 - (count - 1) as f64 * 3.0) / count as f64;
        // A translucent pastel tint lets the native menu material show through.
        let color = meter_color(window, now)
            .blendedColorWithFraction_ofColor(0.35, &NSColor::whiteColor())
            .unwrap_or_else(NSColor::secondaryLabelColor)
            .colorWithAlphaComponent(0.50);
        for segment in 0..count {
            let x = 12.0 + segment as f64 * (width + 3.0);
            view.addSubview(&bar(
                mtm,
                rect(x, bottom + 24.0, width, 5.0),
                &NSColor::secondaryLabelColor().colorWithAlphaComponent(0.10),
            ));
            let fill = (percent * count as f64 / 100.0 - segment as f64).clamp(0.0, 1.0);
            if fill > 0.0 {
                view.addSubview(&bar(mtm, rect(x, bottom + 24.0, width * fill, 5.0), &color));
            }
        }
        let countdown = label(
            mtm,
            &countdown_text(window.resets_at.as_deref(), display.language, now),
            rect(12.0, bottom + 3.0, WIDTH - 24.0, 17.0),
            false,
        );
        countdown.setAlignment(NSTextAlignment::Right);
        countdown.setTextColor(Some(&NSColor::secondaryLabelColor()));
        view.addSubview(&countdown);
        countdowns.push(Countdown {
            label: countdown,
            resets_at: window.resets_at.clone(),
            language: display.language,
        });
    }
    (view, countdowns)
}

fn refresh_content(mtm: MainThreadMarker) {
    CONTENT.with_borrow_mut(|content| {
        if let Some(item) = &content.item {
            // AppKit does not support changing a custom item's size while tracking.
            let open = item.view().and_then(|view| view.window()).is_some();
            if !open {
                if let Some(display) = content.pending.take() {
                    let (view, countdowns) = build_view(&display, mtm);
                    item.setView(Some(&view));
                    content.countdowns = countdowns;
                }
            }
        }
        let now = Utc::now().timestamp();
        for countdown in &content.countdowns {
            countdown
                .label
                .setStringValue(&NSString::from_str(&countdown_text(
                    countdown.resets_at.as_deref(),
                    countdown.language,
                    now,
                )));
        }
    });
}

pub(super) fn update(app: &tauri::AppHandle, display: &TrayDisplayState) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let display = display.clone();
    if let Err(error) = tray.with_inner_tray_icon(move |inner| {
        let mtm = MainThreadMarker::new().expect("tray callback must run on the main thread");
        let Some(menu) = inner.ns_status_item().and_then(|item| item.menu(mtm)) else {
            return;
        };
        CONTENT.with_borrow_mut(|content| {
            if content.item.is_none() {
                let item = NSMenuItem::new(mtm);
                item.setTitle(&NSString::from_str("Usage"));
                menu.insertItem_atIndex(&NSMenuItem::separatorItem(mtm), 0);
                menu.insertItem_atIndex(&item, 0);
                content.item = Some(item);
            }
            content.pending = Some(display);
        });
        refresh_content(mtm);
    }) {
        eprintln!("failed to update native tray menu: {error}");
    }
}

pub(super) fn start_countdown_timer() {
    let callback = RcBlock::new(|_: NonNull<NSTimer>| {
        if let Some(mtm) = MainThreadMarker::new() {
            refresh_content(mtm);
        }
    });
    // The callback captures no AppKit objects; both modes run on the main thread.
    unsafe {
        let timer = NSTimer::timerWithTimeInterval_repeats_block(1.0, true, &callback);
        let run_loop = NSRunLoop::mainRunLoop();
        run_loop.addTimer_forMode(&timer, NSRunLoopCommonModes);
        run_loop.addTimer_forMode(&timer, NSEventTrackingRunLoopMode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_menu_countdown_handles_dates_and_localization() {
        let reset = Some("2026-11-13T06:00:00Z");
        let now = DateTime::parse_from_rfc3339("2026-11-07T00:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(
            countdown_text(reset, TrayLanguage::Zh, now),
            "更新 6天 6小时"
        );
        assert_eq!(
            countdown_text(reset, TrayLanguage::En, now),
            "Updates in 6d 6h"
        );
        assert_eq!(
            countdown_text(reset, TrayLanguage::Zh, now + 7 * 86400),
            "即将更新"
        );
        assert_eq!(countdown_text(None, TrayLanguage::Zh, now), "更新时间未知");
        assert_eq!(
            countdown_text(Some("invalid"), TrayLanguage::En, now),
            "Reset time unavailable"
        );
        assert_eq!(period_label(10080, TrayLanguage::Zh), "每周");
        assert_eq!(period_label(300, TrayLanguage::En), "5 hours");
    }
}

use std::collections::HashSet;

use anyhow::{bail, Context};
use tauri::{
    AppHandle, Emitter, EventTarget, LogicalPosition, LogicalSize, Manager, PhysicalPosition,
    PhysicalSize, WebviewWindow, WindowEvent,
};
use tauri_plugin_log::log;

use crate::save_load::{note_id_from_label, NoteRepository, StoredNote};

const GAP: i32 = 20;
const COLLAPSED_HEIGHT: u32 = 24;
const DEFAULT_NOTE_HEIGHT: u32 = 250;
const DEFAULT_NOTE_WIDTH: u32 = 300;

#[derive(Debug, Clone, Copy)]
struct WindowRect {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

impl WindowRect {
    fn from_physical(position: PhysicalPosition<i32>, size: PhysicalSize<u32>) -> Self {
        Self {
            x: i64::from(position.x),
            y: i64::from(position.y),
            width: i64::from(size.width),
            height: i64::from(size.height),
        }
    }

    fn right(self) -> i64 {
        self.x + self.width
    }

    fn bottom(self) -> i64 {
        self.y + self.height
    }

    fn contains(self, other: Self) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    fn separated_by(self, other: Self, gap: i64) -> bool {
        self.right() + gap <= other.x
            || other.right() + gap <= self.x
            || self.bottom() + gap <= other.y
            || other.bottom() + gap <= self.y
    }

    fn distance_squared_from(self, other: Self) -> i128 {
        let dx = i128::from(2 * self.x + self.width - (2 * other.x + other.width));
        let dy = i128::from(2 * self.y + self.height - (2 * other.y + other.height));
        dx * dx + dy * dy
    }
}

fn nearest_free_position(
    anchor: WindowRect,
    obstacles: &[WindowRect],
    work_area: WindowRect,
    new_size: PhysicalSize<u32>,
) -> Option<PhysicalPosition<i32>> {
    let gap = i64::from(GAP);
    let width = i64::from(new_size.width);
    let height = i64::from(new_size.height);
    let mut positions = vec![
        (anchor.right() + gap, anchor.y),
        (anchor.x, anchor.bottom() + gap),
        (anchor.x - width - gap, anchor.y),
        (anchor.x, anchor.y - height - gap),
    ];
    let mut xs = vec![anchor.x, work_area.x + gap, work_area.right() - width - gap];
    let mut ys = vec![
        anchor.y,
        work_area.y + gap,
        work_area.bottom() - height - gap,
    ];

    for obstacle in obstacles {
        xs.extend([obstacle.x, obstacle.right() + gap, obstacle.x - width - gap]);
        ys.extend([
            obstacle.y,
            obstacle.bottom() + gap,
            obstacle.y - height - gap,
        ]);
    }
    for x in xs {
        for &y in &ys {
            positions.push((x, y));
        }
    }

    let mut seen = HashSet::new();
    let mut candidates: Vec<_> = positions
        .into_iter()
        .enumerate()
        .filter(|(_, position)| seen.insert(*position))
        .filter_map(|(preference, (x, y))| {
            let candidate = WindowRect {
                x,
                y,
                width,
                height,
            };
            if !work_area.contains(candidate)
                || obstacles
                    .iter()
                    .any(|obstacle| !candidate.separated_by(*obstacle, gap))
            {
                return None;
            }
            Some((
                candidate.distance_squared_from(anchor),
                preference,
                candidate,
            ))
        })
        .collect();
    candidates.sort_by_key(|(distance, preference, _)| (*distance, *preference));
    let (_, _, candidate) = candidates.into_iter().next()?;
    Some(PhysicalPosition::new(
        i32::try_from(candidate.x).ok()?,
        i32::try_from(candidate.y).ok()?,
    ))
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

fn get_focused_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.webview_windows()
        .into_iter()
        .find(|(_, window)| window.is_focused().unwrap_or(false))
        .map(|(_label, window)| window)
}

fn get_position_and_size(
    window: &WebviewWindow,
) -> Result<(PhysicalPosition<i32>, PhysicalSize<u32>), anyhow::Error> {
    let window_position = window.outer_position().context(format!(
        "Could not get position of window: {}",
        window.label()
    ))?;
    let window_size = window
        .outer_size()
        .context(format!("Could not get size of window: {}", window.label()))?;
    Ok((window_position, window_size))
}

fn window_overlap(start_1: i32, len_1: i32, start_2: i32, len_2: i32) -> bool {
    let end_1 = start_1 + len_1;
    let end_2 = start_2 + len_2;

    let overlap_start = std::cmp::max(start_1, start_2);
    let overlap_end = std::cmp::min(end_1, end_2);
    overlap_end - overlap_start > GAP
}

pub fn snap_window(
    app: &AppHandle,
    direction: Direction,
    partial: bool,
) -> Result<(), anyhow::Error> {
    log::debug!("Snapping window {:?}", direction);

    let window = get_focused_window(app).context("No window currently focused")?;
    let (window_position, window_size) = get_position_and_size(&window)?;

    let primary_monitor = app
        .primary_monitor()
        .context("could not get primary monitor")?
        .context("no primary monitor")?;

    let active_monitor = app
        .cursor_position()
        .map(|p| p.to_logical(primary_monitor.scale_factor()))
        .and_then(|p| app.monitor_from_point(p.x, p.y))
        .context("could not get cursor position")?
        .context("could not get monitor from cursor position")?;

    let current_monitor = window
        .current_monitor()
        .context(format!(
            "could not find monitor for window to be positioned: {}",
            window.label()
        ))?
        .context("window to be positioned is hidden or otherwise has no display")?;

    if current_monitor.name() != active_monitor.name() {
        window.set_position(
            (PhysicalPosition {
                x: active_monitor.position().x + GAP,
                y: active_monitor.position().y + GAP,
            })
            .to_logical::<i32>(active_monitor.scale_factor()),
        )?;
        return Ok(());
    }

    let other_windows = app
        .webview_windows()
        .into_iter()
        .filter(|(_, wind)| *wind != window)
        .filter_map(|(_, wind)| get_position_and_size(&wind).ok());

    let viable_edges: Box<dyn Iterator<Item = i32>> =
        if partial {
            match direction {
                Direction::Left => Box::new(other_windows.flat_map(|(position, size)| {
                    [position.x + size.width as i32 + GAP, position.x]
                })),
                Direction::Up => Box::new(other_windows.flat_map(|(position, size)| {
                    [position.y + size.height as i32 + GAP, position.y]
                })),
                Direction::Right => Box::new(other_windows.flat_map(|(position, size)| {
                    [
                        (position.x + size.width as i32) - window_size.width as i32,
                        position.x - (window_size.width as i32 + GAP),
                    ]
                })),
                Direction::Down => Box::new(other_windows.flat_map(|(position, size)| {
                    [
                        (position.y + size.height as i32) - window_size.height as i32,
                        position.y - (window_size.height as i32 + GAP),
                    ]
                })),
            }
        } else {
            match direction {
                Direction::Left => Box::new(other_windows.filter_map(|(position, size)| {
                    if window_overlap(
                        position.y,
                        size.height as i32,
                        window_position.y,
                        window_size.height as i32,
                    ) {
                        Some(position.x + size.width as i32 + GAP)
                    } else {
                        None
                    }
                })),
                Direction::Up => Box::new(other_windows.filter_map(|(position, size)| {
                    if window_overlap(
                        position.x,
                        size.width as i32,
                        window_position.x,
                        window_size.width as i32,
                    ) {
                        Some(position.y + size.height as i32 + GAP)
                    } else {
                        None
                    }
                })),
                Direction::Right => Box::new(other_windows.filter_map(|(position, size)| {
                    if window_overlap(
                        position.y,
                        size.height as i32,
                        window_position.y,
                        window_size.height as i32,
                    ) {
                        Some(position.x - (window_size.width as i32 + GAP))
                    } else {
                        None
                    }
                })),
                Direction::Down => Box::new(other_windows.filter_map(|(position, size)| {
                    if window_overlap(
                        position.x,
                        size.width as i32,
                        window_position.x,
                        window_size.width as i32,
                    ) {
                        Some(position.y - (window_size.height as i32 + GAP))
                    } else {
                        None
                    }
                })),
            }
        };

    let position = match direction {
        Direction::Left => PhysicalPosition {
            x: viable_edges
                .filter(|edge| *edge < window_position.x)
                .max()
                .unwrap_or(current_monitor.position().x + GAP),
            y: window_position.y,
        },
        Direction::Up => PhysicalPosition {
            x: window_position.x,
            y: viable_edges
                .filter(|edge| *edge < window_position.y)
                .max()
                .unwrap_or(current_monitor.position().y + GAP),
        },
        Direction::Right => PhysicalPosition {
            x: viable_edges
                .filter(|edge| *edge > window_position.x)
                .min()
                .unwrap_or(
                    ((current_monitor.position().x + current_monitor.size().width as i32)
                        - window_size.width as i32)
                        - GAP,
                ),
            y: window_position.y,
        },
        Direction::Down => PhysicalPosition {
            x: window_position.x,
            y: viable_edges
                .filter(|edge| *edge > window_position.y)
                .min()
                .unwrap_or(
                    ((current_monitor.position().y + current_monitor.size().height as i32)
                        - window_size.height as i32)
                        - GAP,
                ),
        },
    };

    window.set_position(position)?;
    Ok(())
}

pub fn create_sticky(app: &AppHandle) -> Result<WebviewWindow, anyhow::Error> {
    let anchor = get_focused_window(app).or_else(|| sorted_windows(app).into_iter().last());
    let position = anchor
        .as_ref()
        .map(|anchor| -> anyhow::Result<_> {
            let monitor = anchor
                .current_monitor()?
                .or(anchor.primary_monitor()?)
                .context("No monitor available for placing a new note")?;
            let scale_factor = monitor.scale_factor();
            let anchor_rect =
                WindowRect::from_physical(anchor.outer_position()?, anchor.outer_size()?);
            let work_area =
                WindowRect::from_physical(monitor.work_area().position, monitor.work_area().size);
            let new_size =
                LogicalSize::new(DEFAULT_NOTE_WIDTH, DEFAULT_NOTE_HEIGHT).to_physical(scale_factor);
            let obstacles: Vec<_> = app
                .webview_windows()
                .into_values()
                .filter(|window| {
                    window
                        .current_monitor()
                        .ok()
                        .flatten()
                        .is_some_and(|candidate| candidate.name() == monitor.name())
                })
                .filter_map(|window| {
                    get_position_and_size(&window)
                        .ok()
                        .map(|(position, size)| WindowRect::from_physical(position, size))
                })
                .collect();
            nearest_free_position(anchor_rect, &obstacles, work_area, new_size)
                .map(|position| position.to_logical::<i32>(scale_factor))
                .context("No non-overlapping space is available on the current monitor")
        })
        .transpose()?;
    let repository = app.state::<NoteRepository>();
    let note = match position {
        Some(position) => repository.create_at(position.x, position.y)?,
        None => repository.create()?,
    };
    match open_sticky(app, &note) {
        Ok(window) => Ok(window),
        Err(open_error) => {
            repository.delete(&note.id).with_context(|| {
                format!("Could not roll back failed note creation after: {open_error:#}")
            })?;
            Err(open_error.context("Could not open the newly created note"))
        }
    }
}

pub fn focus_existing_or_create(app: &AppHandle) -> Result<(), anyhow::Error> {
    let windows = sorted_windows(app);
    if windows.is_empty() {
        create_sticky(app)?;
        return Ok(());
    }

    for window in &windows {
        window.show()?;
        if window.is_minimized()? {
            window.unminimize()?;
        }
    }
    windows[0].set_focus()?;
    Ok(())
}

pub fn open_sticky(app: &AppHandle, note: &StoredNote) -> Result<WebviewWindow, anyhow::Error> {
    log::debug!("Creating new sticky window");
    let label = format!("sticky_{}", note.id);

    #[derive(serde::Serialize)]
    struct StickyInit<'a> {
        id: &'a str,
        document: &'a serde_json::Value,
        color: &'a str,
        collapsed: bool,
        always_on_top: bool,
    }

    let init = StickyInit {
        id: &note.id,
        document: &note.document,
        color: &note.color,
        collapsed: note.collapsed,
        always_on_top: note.pinned,
    };
    let init_script = format!("window.__STICKY_INIT__ = {}", serde_json::to_string(&init)?);

    let window_height = if note.collapsed {
        COLLAPSED_HEIGHT
    } else {
        note.expanded_height.max(80)
    };
    let builder =
        tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App("index.html".into()))
            .title("Sticky")
            .decorations(false)
            .maximizable(false)
            .resizable(!note.collapsed)
            .visible(true)
            .accept_first_mouse(true)
            .initialization_script(init_script)
            .inner_size(note.expanded_width.max(150) as f64, window_height as f64)
            .position(note.x as f64, note.y as f64)
            .always_on_top(note.pinned)
            .prevent_overflow();

    let window = builder.build().context("Could not create sticky window")?;
    let app_clone = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { .. } = event {
            let _ = cycle_focus(&app_clone, false);
        }
    });

    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWindow;

        let ns_window_ptr = window.ns_window().unwrap();
        unsafe {
            use objc2_app_kit::NSWindowCollectionBehavior;

            let ns_window = &mut *(ns_window_ptr as *mut NSWindow);
            ns_window.setCollectionBehavior(
                NSWindowCollectionBehavior::IgnoresCycle | NSWindowCollectionBehavior::Transient,
            );
        }
    }

    Ok(window)
}

pub fn request_close_sticky(app: &AppHandle) -> Result<(), anyhow::Error> {
    if let Some(window) = get_focused_window(app) {
        window.emit_to(
            EventTarget::webview_window(window.label()),
            "close_note_request",
            (),
        )?;
        Ok(())
    } else {
        bail!("No window currently focused!")
    }
}

pub fn close_window_and_archive(window: &WebviewWindow) -> Result<(), anyhow::Error> {
    let id = note_id_from_label(window.label())?.to_string();
    let repository = window.state::<NoteRepository>();
    let previous_closed_at = repository.get(&id)?.closed_at;
    repository.close(&id)?;
    if let Err(close_error) = window.close() {
        repository
            .update(&id, |note| {
                note.closed_at = previous_closed_at;
                Ok(())
            })
            .with_context(|| {
                format!("Could not roll back failed window close after: {close_error}")
            })?;
        return Err(close_error.into());
    }
    Ok(())
}

pub fn restore_last_closed(app: &AppHandle) -> Result<(), anyhow::Error> {
    let repository = app.state::<NoteRepository>();
    let note = repository
        .restore_last_closed()?
        .context("No recently closed note")?;
    match open_sticky(app, &note) {
        Ok(window) => window.set_focus().context("Could not focus restored note"),
        Err(open_error) => {
            repository.close(&note.id).with_context(|| {
                format!("Could not roll back failed restore after: {open_error:#}")
            })?;
            Err(open_error.context("Could not reopen the last closed note"))
        }
    }
}

pub fn set_window_collapsed(window: &WebviewWindow, collapsed: bool) -> Result<(), anyhow::Error> {
    let id = note_id_from_label(window.label())?;
    let repository = window.state::<NoteRepository>();
    let current = repository.get(id)?;
    if current.collapsed == collapsed {
        return Ok(());
    }

    let scale_factor = window.scale_factor()?;
    let position = window.outer_position()?.to_logical::<i32>(scale_factor);
    let size = window.outer_size()?.to_logical::<u32>(scale_factor);

    if collapsed {
        repository.update(id, |note| {
            note.x = position.x;
            note.y = position.y;
            note.expanded_width = size.width.max(150);
            note.expanded_height = size.height.max(80);
            note.collapsed = true;
            Ok(())
        })?;
        if window.is_maximized()? {
            window.unmaximize()?;
        }
        window.set_resizable(false)?;
        window.set_size(LogicalSize::new(size.width.max(150), COLLAPSED_HEIGHT))?;
        return Ok(());
    }

    let monitor = window
        .current_monitor()?
        .or(window.primary_monitor()?)
        .context("No active monitor available for expanding note")?;
    let monitor_scale = monitor.scale_factor();
    let monitor_position = monitor.position().to_logical::<i32>(monitor_scale);
    let monitor_size = monitor.size().to_logical::<u32>(monitor_scale);
    let width = current
        .expanded_width
        .clamp(150, monitor_size.width.max(150));
    let height = current
        .expanded_height
        .clamp(80, monitor_size.height.max(80));
    let max_x = monitor_position.x + monitor_size.width.saturating_sub(width) as i32;
    let max_y = monitor_position.y + monitor_size.height.saturating_sub(height) as i32;
    let x = position.x.clamp(monitor_position.x, max_x);
    let y = position.y.clamp(monitor_position.y, max_y);

    window.set_resizable(true)?;
    window.set_size(LogicalSize::new(width, height))?;
    window.set_position(LogicalPosition::new(x, y))?;
    repository.update(id, |note| {
        note.x = x;
        note.y = y;
        note.collapsed = false;
        Ok(())
    })?;
    Ok(())
}

pub fn sorted_windows(app: &AppHandle) -> Vec<WebviewWindow> {
    let mut positions: Vec<_> = app
        .webview_windows()
        .into_iter()
        .filter_map(|(_label, w)| get_position_and_size(&w).ok().map(|(p, _)| (p, w)))
        .collect();

    positions.sort_by_key(|(p, _)| *p);

    positions.into_iter().map(|(_, w)| w).collect()
}

pub fn cycle_focus(app: &AppHandle, reverse: bool) -> Result<(), anyhow::Error> {
    let mut sorted_windows = sorted_windows(app);
    if reverse {
        sorted_windows.reverse();
    }

    let focused_index = sorted_windows
        .iter()
        .position(|w| w.is_focused().unwrap_or(false))
        .context("No window currently focused")?;

    let next_window_index = (focused_index + 1) % sorted_windows.len();

    sorted_windows[next_window_index]
        .set_focus()
        .context("Could not focus window")
}

pub fn fit_text(app: &AppHandle) -> Result<(), anyhow::Error> {
    app.webview_windows()
        .into_iter()
        .for_each(|(label, window)| {
            if window.is_focused().unwrap_or(false) {
                log::info!("emitting fit_text to window {}", label);
                let _ = window.emit_to(EventTarget::webview_window(label), "fit_text", ());
            }
        });

    Ok(())
}

pub fn set_color(app: &AppHandle, index: u8) -> Result<(), anyhow::Error> {
    app.webview_windows()
        .into_iter()
        .for_each(|(label, window)| {
            if window.is_focused().unwrap_or(false) {
                log::info!("emitting set color to window {}", label);
                let _ = window.emit_to(EventTarget::webview_window(label), "set_color", index);
            }
        });

    Ok(())
}

pub fn reset_note_positions(app: &AppHandle) -> anyhow::Result<()> {
    app.webview_windows().into_values().try_for_each(|window| {
        window
            .set_position(PhysicalPosition { x: 0, y: 0 })
            .context("could not set note position")
    })
}

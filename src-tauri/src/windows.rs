#[cfg(any(not(target_os = "macos"), test))]
use std::collections::BTreeMap;
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use anyhow::{bail, Context};
use tauri::{
    AppHandle, Emitter, EventTarget, LogicalPosition, LogicalSize, Manager, PhysicalPosition,
    PhysicalSize, WebviewWindow, WindowEvent,
};
use tauri_plugin_log::log;

use crate::save_load::{note_id_from_label, NoteRepository, StoredNote};

const GAP: i32 = 20;
const STACK_GAP: i32 = 12;
const COLLAPSED_HEIGHT: u32 = 24;
const DEFAULT_NOTE_HEIGHT: u32 = 250;
const DEFAULT_NOTE_WIDTH: u32 = 300;
const SHORTCUTS_WINDOW_LABEL: &str = "keyboard_shortcuts";

#[derive(Debug, Clone)]
struct ActiveDrag {
    #[cfg(not(target_os = "macos"))]
    leader: String,
    #[cfg(not(target_os = "macos"))]
    leader_start: PhysicalPosition<i32>,
    #[cfg(not(target_os = "macos"))]
    starting_positions: BTreeMap<String, PhysicalPosition<i32>>,
}

#[derive(Default)]
pub struct DragCoordinator(Mutex<Option<ActiveDrag>>);

#[derive(Default)]
pub struct NoteVisibility(Mutex<Option<String>>);

impl DragCoordinator {
    pub fn begin(
        &self,
        app: &AppHandle,
        leader: &WebviewWindow,
    ) -> anyhow::Result<Vec<WebviewWindow>> {
        self.finish()?;
        let leader_id = note_id_from_label(leader.label())?;
        let Some(order) = app.state::<NoteRepository>().linked_stack()? else {
            return Ok(Vec::new());
        };
        if !order.iter().any(|id| id == leader_id) {
            return Ok(Vec::new());
        }

        let windows: HashMap<_, _> = app.webview_windows().into_iter().collect();
        #[cfg(not(target_os = "macos"))]
        let mut starting_positions = BTreeMap::new();
        let mut linked_windows = Vec::new();
        for id in order {
            let label = format!("sticky_{id}");
            if let Some(window) = windows.get(&label) {
                #[cfg(not(target_os = "macos"))]
                starting_positions.insert(label, window.outer_position()?);
                if window.label() != leader.label() {
                    linked_windows.push(window.clone());
                }
            }
        }
        if !windows.contains_key(leader.label()) {
            bail!("Dragged note was not among the open linked notes");
        }
        #[cfg(not(target_os = "macos"))]
        let leader_start = *starting_positions
            .get(leader.label())
            .context("Dragged note was not among the open linked notes")?;
        let active = ActiveDrag {
            #[cfg(not(target_os = "macos"))]
            leader: leader.label().to_string(),
            #[cfg(not(target_os = "macos"))]
            leader_start,
            #[cfg(not(target_os = "macos"))]
            starting_positions,
        };
        *self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Drag coordinator lock poisoned"))? = Some(active);
        Ok(linked_windows)
    }

    #[cfg(not(target_os = "macos"))]
    fn movement(
        &self,
        leader: &str,
        position: PhysicalPosition<i32>,
    ) -> anyhow::Result<Option<(ActiveDrag, Vec<(String, PhysicalPosition<i32>)>)>> {
        let guard = self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Drag coordinator lock poisoned"))?;
        let Some(active) = guard.as_ref().filter(|active| active.leader == leader) else {
            return Ok(None);
        };
        let targets =
            translated_positions(&active.starting_positions, active.leader_start, position)?;
        Ok(Some((active.clone(), targets)))
    }

    pub fn finish(&self) -> anyhow::Result<()> {
        *self
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Drag coordinator lock poisoned"))? = None;
        Ok(())
    }
}

fn capture_stack_order(mut positions: Vec<(String, i32, i32)>) -> Vec<String> {
    positions.sort_by(|a, b| (a.2, a.1, &a.0).cmp(&(b.2, b.1, &b.0)));
    positions.into_iter().map(|(id, _, _)| id).collect()
}

fn vertical_stack_positions(
    origin: PhysicalPosition<i32>,
    count: usize,
    step: i32,
) -> anyhow::Result<Vec<PhysicalPosition<i32>>> {
    let mut y = i64::from(origin.y);
    (0..count)
        .map(|_| {
            let position = PhysicalPosition::new(
                origin.x,
                i32::try_from(y).context("Linked note position exceeded platform limits")?,
            );
            y = y
                .checked_add(i64::from(step))
                .context("Linked note stack height overflowed")?;
            Ok(position)
        })
        .collect()
}

fn physical_stack_step(window: &WebviewWindow) -> anyhow::Result<i32> {
    let step = f64::from(COLLAPSED_HEIGHT + STACK_GAP as u32) * window.scale_factor()?;
    if !step.is_finite() || step > f64::from(i32::MAX) {
        bail!("Linked note scale produced an invalid stack step");
    }
    Ok(step.round() as i32)
}

#[cfg(any(not(target_os = "macos"), test))]
fn translated_positions(
    starts: &BTreeMap<String, PhysicalPosition<i32>>,
    leader_start: PhysicalPosition<i32>,
    leader_current: PhysicalPosition<i32>,
) -> anyhow::Result<Vec<(String, PhysicalPosition<i32>)>> {
    let dx = i64::from(leader_current.x) - i64::from(leader_start.x);
    let dy = i64::from(leader_current.y) - i64::from(leader_start.y);
    starts
        .iter()
        .map(|(label, start)| {
            let x = i32::try_from(i64::from(start.x) + dx)
                .context("Linked note drag exceeded horizontal platform limits")?;
            let y = i32::try_from(i64::from(start.y) + dy)
                .context("Linked note drag exceeded vertical platform limits")?;
            Ok((label.clone(), PhysicalPosition::new(x, y)))
        })
        .collect()
}

struct WindowSnapshot {
    window: WebviewWindow,
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
}

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
        .find(|(label, window)| {
            label.starts_with("sticky_") && window.is_focused().unwrap_or(false)
        })
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

fn linked_window_snapshots(app: &AppHandle) -> anyhow::Result<Option<Vec<WindowSnapshot>>> {
    let repository = app.state::<NoteRepository>();
    let Some(order) = repository.linked_stack()? else {
        return Ok(None);
    };
    let windows: HashMap<_, _> = app.webview_windows().into_iter().collect();
    let mut snapshots = Vec::new();
    for id in order {
        if repository.get(&id)?.closed_at.is_some() {
            continue;
        }
        let label = format!("sticky_{id}");
        let Some(window) = windows.get(&label) else {
            continue;
        };
        let position = window.outer_position()?;
        let size = window.outer_size()?;
        snapshots.push(WindowSnapshot {
            window: window.clone(),
            position,
            size,
        });
    }
    Ok(Some(snapshots))
}

fn restore_positions(snapshots: &[WindowSnapshot]) {
    for snapshot in snapshots {
        if let Err(error) = snapshot.window.set_position(snapshot.position) {
            log::error!(
                "Could not restore linked note {} after positioning failure: {error}",
                snapshot.window.label()
            );
        }
    }
}

fn restore_geometry(snapshots: &[WindowSnapshot]) {
    for snapshot in snapshots {
        if let Err(error) = snapshot.window.set_size(snapshot.size) {
            log::error!(
                "Could not restore linked note {} size after arranging failure: {error}",
                snapshot.window.label()
            );
        }
        if let Err(error) = snapshot.window.set_position(snapshot.position) {
            log::error!(
                "Could not restore linked note {} position after arranging failure: {error}",
                snapshot.window.label()
            );
        }
    }
}

fn reset_snapshot_widths(snapshots: &[WindowSnapshot]) -> anyhow::Result<()> {
    let heights = snapshots
        .iter()
        .map(|snapshot| {
            let scale_factor = snapshot.window.scale_factor()?;
            Ok(snapshot.size.to_logical::<u32>(scale_factor).height)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    for (snapshot, height) in snapshots.iter().zip(heights) {
        if let Err(error) = snapshot
            .window
            .set_size(LogicalSize::new(DEFAULT_NOTE_WIDTH, height))
        {
            restore_geometry(snapshots);
            return Err(error).with_context(|| {
                format!(
                    "Could not reset linked note {} width",
                    snapshot.window.label()
                )
            });
        }
    }
    Ok(())
}

fn move_snapshots(
    snapshots: &[WindowSnapshot],
    targets: &[PhysicalPosition<i32>],
) -> anyhow::Result<()> {
    if snapshots.len() != targets.len() {
        bail!("Linked note target count did not match the open note count");
    }
    for (index, (snapshot, target)) in snapshots.iter().zip(targets).enumerate() {
        if let Err(error) = snapshot.window.set_position(*target) {
            restore_positions(&snapshots[..index]);
            return Err(error).with_context(|| {
                format!("Could not position linked note {}", snapshot.window.label())
            });
        }
    }
    Ok(())
}

fn reflow_linked_stack_from_horizontal_anchor(
    app: &AppHandle,
    horizontal_anchor: Option<&str>,
) -> anyhow::Result<()> {
    let Some(snapshots) = linked_window_snapshots(app)? else {
        return Ok(());
    };
    let Some(first) = snapshots.first() else {
        return Ok(());
    };
    let origin = PhysicalPosition::new(
        horizontal_anchor
            .and_then(|label| {
                snapshots
                    .iter()
                    .find(|snapshot| snapshot.window.label() == label)
            })
            .map_or(first.position.x, |snapshot| snapshot.position.x),
        first.position.y,
    );
    let targets =
        vertical_stack_positions(origin, snapshots.len(), physical_stack_step(&first.window)?)?;
    move_snapshots(&snapshots, &targets)
}

pub fn reflow_linked_stack(app: &AppHandle) -> anyhow::Result<()> {
    reflow_linked_stack_from_horizontal_anchor(app, None)
}

pub fn arrange_and_link_all_notes(app: &AppHandle) -> anyhow::Result<()> {
    let repository = app.state::<NoteRepository>();
    let windows: HashMap<_, _> = app.webview_windows().into_iter().collect();
    let mut positions = Vec::new();
    for note in repository.all()? {
        let label = format!("sticky_{}", note.id);
        let (x, y) = if let Some(window) = windows.get(&label) {
            let scale_factor = window.scale_factor()?;
            let position = window.outer_position()?.to_logical::<i32>(scale_factor);
            (position.x, position.y)
        } else {
            (note.x, note.y)
        };
        positions.push((note.id, x, y));
    }
    let order = capture_stack_order(positions);
    let stack_origin = order
        .iter()
        .find_map(|id| windows.get(&format!("sticky_{id}")))
        .map(|window| {
            Ok::<_, anyhow::Error>((window.outer_position()?, physical_stack_step(window)?))
        })
        .transpose()?;

    let mut snapshots = Vec::new();
    for id in &order {
        if let Some(window) = windows.get(&format!("sticky_{id}")) {
            let position = window.outer_position()?;
            let size = window.outer_size()?;
            snapshots.push(WindowSnapshot {
                window: window.clone(),
                position,
                size,
            });
        }
    }
    let resized_ids = snapshots
        .iter()
        .map(|snapshot| note_id_from_label(snapshot.window.label()).map(str::to_owned))
        .collect::<anyhow::Result<Vec<_>>>()?;
    if let Some((origin, step)) = stack_origin {
        let targets = vertical_stack_positions(origin, snapshots.len(), step)?;
        reset_snapshot_widths(&snapshots)?;
        if let Err(error) = move_snapshots(&snapshots, &targets) {
            restore_geometry(&snapshots);
            return Err(error);
        }
    }

    if let Err(error) =
        repository.set_linked_stack_and_widths(order, &resized_ids, DEFAULT_NOTE_WIDTH)
    {
        restore_geometry(&snapshots);
        return Err(error).context("Could not persist linked note arrangement");
    }
    Ok(())
}

pub fn unlink_notes(app: &AppHandle) -> anyhow::Result<()> {
    app.state::<NoteRepository>().set_linked_stack(None)?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn move_linked_notes_for_drag(
    app: &AppHandle,
    leader: &str,
    position: PhysicalPosition<i32>,
) -> anyhow::Result<()> {
    let coordinator = app.state::<DragCoordinator>();
    let Some((active, targets)) = coordinator.movement(leader, position)? else {
        return Ok(());
    };
    let windows: HashMap<_, _> = app.webview_windows().into_iter().collect();
    let mut moved = Vec::new();
    for (label, target) in targets {
        if label == leader {
            continue;
        }
        let Some(window) = windows.get(&label) else {
            continue;
        };
        if let Err(error) = window.set_position(target) {
            for moved_label in moved {
                if let (Some(window), Some(start)) = (
                    windows.get(&moved_label),
                    active.starting_positions.get(&moved_label),
                ) {
                    let _ = window.set_position(*start);
                }
            }
            if let Some(window) = windows.get(leader) {
                let _ = window.set_position(active.leader_start);
            }
            coordinator.finish()?;
            return Err(error).with_context(|| format!("Could not move linked note {label}"));
        }
        moved.push(label);
    }
    Ok(())
}

fn linked_new_note_position(app: &AppHandle) -> anyhow::Result<Option<LogicalPosition<i32>>> {
    let repository = app.state::<NoteRepository>();
    let Some(order) = repository.linked_stack()? else {
        return Ok(None);
    };
    let windows: HashMap<_, _> = app.webview_windows().into_iter().collect();
    for id in order.iter().rev() {
        let Some(window) = windows.get(&format!("sticky_{id}")) else {
            continue;
        };
        let scale_factor = window.scale_factor()?;
        let position = window.outer_position()?.to_logical::<i32>(scale_factor);
        let y = i32::try_from(
            i64::from(position.y) + i64::from(COLLAPSED_HEIGHT) + i64::from(STACK_GAP),
        )
        .context("New linked note position exceeded platform limits")?;
        return Ok(Some(LogicalPosition::new(position.x, y)));
    }
    Ok(Some(LogicalPosition::new(0, 0)))
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
    let linked_position = linked_new_note_position(app)?;
    let position = if linked_position.is_some() {
        linked_position
    } else {
        let anchor = get_focused_window(app).or_else(|| sorted_windows(app).into_iter().last());
        anchor
            .as_ref()
            .map(|anchor| -> anyhow::Result<_> {
                let monitor = anchor
                    .current_monitor()?
                    .or(anchor.primary_monitor()?)
                    .context("No monitor available for placing a new note")?;
                let scale_factor = monitor.scale_factor();
                let anchor_rect =
                    WindowRect::from_physical(anchor.outer_position()?, anchor.outer_size()?);
                let work_area = WindowRect::from_physical(
                    monitor.work_area().position,
                    monitor.work_area().size,
                );
                let new_size = LogicalSize::new(DEFAULT_NOTE_WIDTH, DEFAULT_NOTE_HEIGHT)
                    .to_physical(scale_factor);
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
            .transpose()?
    };
    let repository = app.state::<NoteRepository>();
    let note = match position {
        Some(position) => repository.create_at(position.x, position.y)?,
        None => repository.create()?,
    };
    match open_sticky(app, &note) {
        Ok(window) => {
            reflow_linked_stack(app)?;
            Ok(window)
        }
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
    let window_label = window.label().to_string();
    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { .. } => {
            let _ = cycle_focus(&app_clone, false);
        }
        #[cfg(not(target_os = "macos"))]
        WindowEvent::Moved(position) => {
            if let Err(error) = move_linked_notes_for_drag(&app_clone, &window_label, *position) {
                log::error!("Could not move linked note stack: {error:#}");
            }
        }
        WindowEvent::Resized(_) => {
            if let Err(error) =
                reflow_linked_stack_from_horizontal_anchor(&app_clone, Some(&window_label))
            {
                log::error!("Could not reflow resized linked note stack: {error:#}");
            }
        }
        _ => {}
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
    if let Some(window) = app.get_webview_window(SHORTCUTS_WINDOW_LABEL) {
        if window.is_focused()? {
            window.close()?;
            return Ok(());
        }
    }
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
    reflow_linked_stack(window.app_handle())?;
    Ok(())
}

pub fn restore_last_closed(app: &AppHandle) -> Result<(), anyhow::Error> {
    let repository = app.state::<NoteRepository>();
    let note = repository
        .restore_last_closed()?
        .context("No recently closed note")?;
    match open_sticky(app, &note) {
        Ok(window) => {
            reflow_linked_stack(app)?;
            window.set_focus().context("Could not focus restored note")
        }
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
        reflow_linked_stack(window.app_handle())?;
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
    reflow_linked_stack(window.app_handle())?;
    window.set_focus()?;
    Ok(())
}

pub fn sorted_windows(app: &AppHandle) -> Vec<WebviewWindow> {
    let mut positions: Vec<_> = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| label.starts_with("sticky_"))
        .filter_map(|(_label, w)| get_position_and_size(&w).ok().map(|(p, _)| (p, w)))
        .collect();

    positions.sort_by_key(|(p, _)| *p);

    positions.into_iter().map(|(_, w)| w).collect()
}

pub fn toggle_shortcuts_window(app: &AppHandle) -> Result<(), anyhow::Error> {
    if let Some(window) = app.get_webview_window(SHORTCUTS_WINDOW_LABEL) {
        window.close()?;
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        app,
        SHORTCUTS_WINDOW_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("Keyboard Shortcuts")
    .initialization_script("window.__SHORTCUTS__ = true")
    .inner_size(440.0, 560.0)
    .resizable(false)
    .maximizable(false)
    .always_on_top(true)
    .center()
    .build()?
    .set_focus()?;
    Ok(())
}

pub fn toggle_note_visibility(app: &AppHandle) -> Result<(), anyhow::Error> {
    let windows = sorted_windows(app);
    let should_hide = windows.iter().try_fold(false, |visible, window| {
        window.is_visible().map(|is_visible| visible || is_visible)
    })?;

    if should_hide {
        let focused_label = windows
            .iter()
            .find(|window| window.is_focused().unwrap_or(false))
            .map(|window| window.label().to_string());
        *app.state::<NoteVisibility>()
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Note visibility lock poisoned"))? = focused_label;
        for window in windows {
            window.hide()?;
        }
    } else if !windows.is_empty() {
        for window in &windows {
            window.show()?;
            if window.is_minimized()? {
                window.unminimize()?;
            }
        }
        let focused_label = app
            .state::<NoteVisibility>()
            .0
            .lock()
            .map_err(|_| anyhow::anyhow!("Note visibility lock poisoned"))?
            .take();
        focused_label
            .and_then(|label| app.get_webview_window(&label))
            .unwrap_or_else(|| windows[0].clone())
            .set_focus()?;
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_order_is_captured_top_to_bottom_then_left_to_right() {
        let order = capture_stack_order(vec![
            ("right".into(), 300, 20),
            ("bottom".into(), 10, 80),
            ("left".into(), 100, 20),
        ]);

        assert_eq!(order, vec!["left", "right", "bottom"]);
    }

    #[test]
    fn vertical_stack_positions_keep_fixed_titlebar_spacing() {
        let positions = vertical_stack_positions(PhysicalPosition::new(40, 20), 3, 36).unwrap();

        assert_eq!(
            positions,
            vec![
                PhysicalPosition::new(40, 20),
                PhysicalPosition::new(40, 56),
                PhysicalPosition::new(40, 92),
            ]
        );
    }

    #[test]
    fn linked_drag_applies_the_leaders_delta_to_every_starting_position() {
        let starts = BTreeMap::from([
            ("leader".into(), PhysicalPosition::new(100, 200)),
            ("other".into(), PhysicalPosition::new(40, 500)),
        ]);
        let positions = translated_positions(
            &starts,
            PhysicalPosition::new(100, 200),
            PhysicalPosition::new(125, 175),
        )
        .unwrap();

        assert_eq!(
            positions,
            vec![
                ("leader".into(), PhysicalPosition::new(125, 175)),
                ("other".into(), PhysicalPosition::new(65, 475)),
            ]
        );
    }
}

use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, MutexGuard},
};

use anyhow::{bail, Context};
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewWindow,
};
use tauri_plugin_log::log;

use crate::{
    save_load::{
        current_time_millis, note_id_from_label, GroupMemberKind, NoteRepository, StoredGroup,
        StoredGroupMember, StoredNote,
    },
    timers::{
        close_timer, remove_timer_for_close, restore_timer_after_failed_close,
        set_ungrouped_timer_collapsed, StoredTimer, TimerRepository, TIMER_HEIGHT, TIMER_WIDTH,
    },
    windows::{
        close_ungrouped_window_and_archive, open_missing_active_notes, open_sticky,
        set_ungrouped_window_collapsed, sorted_windows, GeometryIndex, NoteGeometry,
    },
};

const COLLAPSED_HEIGHT: u32 = 24;
const GROUP_GAP: u32 = 12;
const RESET_MARGIN: i32 = 20;
const DRAG_DETACH_THRESHOLD: i32 = 4;
const NATIVE_POSITION_ROUNDING_TOLERANCE: i32 = 2;

#[derive(Default)]
pub(crate) struct GroupRuntimeState {
    drag_origins: HashMap<String, PhysicalPosition<i32>>,
    completed_drag_origins: HashMap<String, PhysicalPosition<i32>>,
    programmatic_positions: HashMap<String, PhysicalPosition<i32>>,
}

impl GroupRuntimeState {
    fn begin_user_drag(&mut self, id: String, origin: PhysicalPosition<i32>) {
        self.completed_drag_origins.remove(&id);
        self.programmatic_positions.remove(&id);
        self.drag_origins.insert(id, origin);
    }

    fn cancel_user_drag(&mut self, id: &str) {
        self.drag_origins.remove(id);
    }

    fn complete_user_drag(&mut self, id: &str) -> anyhow::Result<()> {
        let origin = self
            .drag_origins
            .remove(id)
            .with_context(|| format!("No active drag origin for window {id}"))?;
        self.completed_drag_origins.insert(id.to_string(), origin);
        Ok(())
    }

    fn take_completed_drag(&mut self, id: &str) -> Option<PhysicalPosition<i32>> {
        self.completed_drag_origins.remove(id)
    }

    fn record_programmatic_position(&mut self, id: String, position: PhysicalPosition<i32>) {
        self.completed_drag_origins.remove(&id);
        self.programmatic_positions.insert(id, position);
    }
}

#[derive(Default)]
pub struct GroupRuntime(Mutex<GroupRuntimeState>);

impl GroupRuntime {
    pub(crate) fn lock(&self) -> anyhow::Result<MutexGuard<'_, GroupRuntimeState>> {
        self.0
            .lock()
            .map_err(|_| anyhow::anyhow!("Group runtime lock poisoned"))
    }
}

struct WindowSnapshot {
    member: StoredGroupMember,
    window: WebviewWindow,
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
}

struct GroupLayout {
    snapshots: Vec<WindowSnapshot>,
    targets: Vec<LogicalPosition<i32>>,
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

    fn center_twice(self) -> (i64, i64) {
        (2 * self.x + self.width, 2 * self.y + self.height)
    }

    fn contains_center_twice(self, (x, y): (i64, i64)) -> bool {
        2 * self.x <= x && x < 2 * self.right() && 2 * self.y <= y && y < 2 * self.bottom()
    }
}

fn get_focused_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.webview_windows()
        .into_iter()
        .find(|(label, window)| {
            (label.starts_with("sticky_") || label.starts_with("timer_"))
                && window.is_focused().unwrap_or(false)
        })
        .map(|(_, window)| window)
}

fn snapshots_for_members(
    app: &AppHandle,
    members: &[StoredGroupMember],
) -> anyhow::Result<Vec<WindowSnapshot>> {
    let windows = app.webview_windows();
    let geometries = app.state::<GeometryIndex>();
    members
        .iter()
        .map(|member| {
            let window = windows.get(&member.window_label()).with_context(|| {
                format!("Active window {:?} did not have an open window", member)
            })?;
            let geometry = geometries.get(&member.id)?;
            Ok(WindowSnapshot {
                member: member.clone(),
                window: window.clone(),
                position: geometry.position,
                size: geometry.size,
            })
        })
        .collect()
}

fn visual_order(
    members: &HashSet<StoredGroupMember>,
    geometries: &GeometryIndex,
) -> anyhow::Result<Vec<StoredGroupMember>> {
    let mut ordered = members
        .iter()
        .map(|member| Ok((member.clone(), geometries.get(&member.id)?)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    ordered.sort_by(|a, b| {
        let a_position = a.1.position;
        let b_position = b.1.position;
        (a_position.y, a_position.x, &a.0).cmp(&(b_position.y, b_position.x, &b.0))
    });
    Ok(ordered.into_iter().map(|(id, _)| id).collect())
}

fn members_on_anchor_monitor_side(
    anchor: &StoredGroupMember,
    candidates: &[StoredGroupMember],
    geometries: &GeometryIndex,
    monitor: WindowRect,
    work_area: WindowRect,
) -> anyhow::Result<Vec<StoredGroupMember>> {
    let anchor_geometry = geometries.get(&anchor.id)?;
    let anchor_center =
        WindowRect::from_physical(anchor_geometry.position, anchor_geometry.size).center_twice();
    if !monitor.contains_center_twice(anchor_center) {
        bail!("Selected parent center was outside its current monitor");
    }
    let midpoint_twice = 2 * work_area.x + work_area.width;
    let anchor_is_left = anchor_center.0 < midpoint_twice;
    let mut eligible = HashSet::new();
    for member in candidates {
        let geometry = geometries.get(&member.id)?;
        let center = WindowRect::from_physical(geometry.position, geometry.size).center_twice();
        if monitor.contains_center_twice(center) && (center.0 < midpoint_twice) == anchor_is_left {
            eligible.insert(member.clone());
        }
    }
    visual_order(&eligible, geometries)
}

fn durable_relink_order(
    parent: &StoredGroupMember,
    active_order: &[StoredGroupMember],
    absorbed_groups: &[StoredGroup],
) -> Vec<StoredGroupMember> {
    let active_members: HashSet<_> = active_order.iter().cloned().collect();
    let mut inactive_before: HashMap<StoredGroupMember, Vec<StoredGroupMember>> = HashMap::new();
    let mut inactive_after: HashMap<StoredGroupMember, Vec<StoredGroupMember>> = HashMap::new();

    for group in absorbed_groups {
        let Some(first_active) = group
            .members
            .iter()
            .find(|member| active_members.contains(*member))
            .cloned()
        else {
            continue;
        };
        let mut previous_active = None;
        for member in &group.members {
            if active_members.contains(member) {
                previous_active = Some(member.clone());
            } else if let Some(active) = &previous_active {
                inactive_after
                    .entry(active.clone())
                    .or_default()
                    .push(member.clone());
            } else {
                inactive_before
                    .entry(first_active.clone())
                    .or_default()
                    .push(member.clone());
            }
        }
    }

    let mut durable_order = Vec::new();
    for active in active_order {
        if active == parent {
            durable_order.push(active.clone());
            durable_order.extend(inactive_before.remove(active).unwrap_or_default());
        } else {
            durable_order.extend(inactive_before.remove(active).unwrap_or_default());
            durable_order.push(active.clone());
        }
        durable_order.extend(inactive_after.remove(active).unwrap_or_default());
    }
    durable_order
}

fn persist_relinked_group(
    store: &mut crate::save_load::NoteStore,
    absorbed_group_ids: &HashSet<String>,
    group_id: &str,
    members: &[StoredGroupMember],
) {
    for absorbed_group_id in absorbed_group_ids {
        store.groups.remove(absorbed_group_id);
    }
    store.groups.insert(
        group_id.to_string(),
        StoredGroup {
            id: group_id.to_string(),
            members: members.to_vec(),
        },
    );
}

fn arranged_positions(
    origin: LogicalPosition<i32>,
    heights: &[u32],
) -> anyhow::Result<Vec<LogicalPosition<i32>>> {
    let mut y = i64::from(origin.y);
    heights
        .iter()
        .enumerate()
        .map(|(index, height)| {
            let position = LogicalPosition::new(
                origin.x,
                i32::try_from(y).context("Group position exceeded platform limits")?,
            );
            if index + 1 < heights.len() {
                y = y
                    .checked_add(i64::from(*height) + i64::from(GROUP_GAP))
                    .context("Group layout height overflowed")?;
            }
            Ok(position)
        })
        .collect()
}

#[derive(Debug, Clone)]
enum StoredSurface {
    Note(StoredNote),
    Timer(StoredTimer),
}

impl StoredSurface {
    fn member(&self) -> StoredGroupMember {
        match self {
            Self::Note(note) => StoredGroupMember::note(&note.id),
            Self::Timer(timer) => StoredGroupMember::timer(&timer.id),
        }
    }

    fn x(&self) -> i32 {
        match self {
            Self::Note(note) => note.x,
            Self::Timer(timer) => timer.x,
        }
    }

    fn y(&self) -> i32 {
        match self {
            Self::Note(note) => note.y,
            Self::Timer(timer) => timer.y,
        }
    }

    fn collapsed(&self) -> bool {
        match self {
            Self::Note(note) => note.collapsed,
            Self::Timer(timer) => timer.collapsed,
        }
    }

    fn pinned(&self) -> bool {
        match self {
            Self::Note(note) => note.pinned,
            Self::Timer(timer) => timer.pinned,
        }
    }

    fn width(&self) -> u32 {
        match self {
            Self::Note(note) => note.expanded_width.max(150),
            Self::Timer(_) => TIMER_WIDTH,
        }
    }

    fn expanded_height(&self) -> u32 {
        match self {
            Self::Note(note) => note.expanded_height.max(80),
            Self::Timer(_) => TIMER_HEIGHT,
        }
    }

    fn durable_height(&self) -> u32 {
        if self.collapsed() {
            COLLAPSED_HEIGHT
        } else {
            self.expanded_height()
        }
    }

    fn set_position(&mut self, x: i32, y: i32) {
        match self {
            Self::Note(note) => {
                note.x = x;
                note.y = y;
            }
            Self::Timer(timer) => {
                timer.x = x;
                timer.y = y;
            }
        }
    }

    fn set_collapsed_and_size(&mut self, collapsed: bool, size: LogicalSize<u32>) {
        match self {
            Self::Note(note) => {
                if collapsed {
                    note.expanded_width = size.width.max(150);
                } else if !note.collapsed {
                    note.expanded_width = size.width.max(150);
                    note.expanded_height = size.height.max(80);
                }
                note.collapsed = collapsed;
            }
            Self::Timer(timer) => timer.collapsed = collapsed,
        }
    }
}

fn stored_surface(app: &AppHandle, member: &StoredGroupMember) -> anyhow::Result<StoredSurface> {
    match member.kind {
        GroupMemberKind::Note => app
            .state::<NoteRepository>()
            .get(&member.id)
            .map(StoredSurface::Note),
        GroupMemberKind::Timer => app
            .state::<TimerRepository>()
            .get(&member.id)
            .map(StoredSurface::Timer),
    }
}

fn replace_surface_batch(app: &AppHandle, surfaces: &[StoredSurface]) -> anyhow::Result<()> {
    let notes = surfaces
        .iter()
        .filter_map(|surface| match surface {
            StoredSurface::Note(note) => Some(note.clone()),
            StoredSurface::Timer(_) => None,
        })
        .collect::<Vec<_>>();
    let timers = surfaces
        .iter()
        .filter_map(|surface| match surface {
            StoredSurface::Note(_) => None,
            StoredSurface::Timer(timer) => Some(timer.clone()),
        })
        .collect::<Vec<_>>();

    if !timers.is_empty() {
        app.state::<TimerRepository>().mutate(|stored| {
            for replacement in &timers {
                *stored.get_mut(&replacement.id).with_context(|| {
                    format!("Cannot replace missing timer {}", replacement.id)
                })? = replacement.clone();
            }
            Ok(())
        })?;
    }
    if !notes.is_empty() {
        app.state::<NoteRepository>().mutate(|store| {
            for replacement in &notes {
                *store
                    .notes
                    .get_mut(&replacement.id)
                    .with_context(|| format!("Cannot replace missing note {}", replacement.id))? =
                    replacement.clone();
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn persist_surface_changes(
    app: &AppHandle,
    replacements: &[StoredSurface],
) -> anyhow::Result<Vec<StoredSurface>> {
    let mut originals = Vec::with_capacity(replacements.len());
    for replacement in replacements {
        originals.push(stored_surface(app, &replacement.member())?);
    }
    if let Err(error) = replace_surface_batch(app, replacements) {
        if let Err(rollback) = replace_surface_batch(app, &originals) {
            log::error!("Could not roll back durable linked-window geometry: {rollback:#}");
        }
        return Err(error);
    }
    Ok(originals)
}

fn restore_surface_changes(app: &AppHandle, originals: &[StoredSurface]) {
    if let Err(error) = replace_surface_batch(app, originals) {
        log::error!("Could not roll back durable linked-window state: {error:#}");
    }
}

fn positions_after_changed_note(
    origin: LogicalPosition<i32>,
    changed_height: u32,
    later_heights: &[u32],
) -> anyhow::Result<Vec<LogicalPosition<i32>>> {
    let heights = std::iter::once(changed_height)
        .chain(later_heights.iter().copied())
        .collect::<Vec<_>>();
    Ok(arranged_positions(origin, &heights)?
        .into_iter()
        .skip(1)
        .collect())
}

fn requested_physical_position(
    snapshot: &WindowSnapshot,
    target: LogicalPosition<i32>,
) -> anyhow::Result<PhysicalPosition<i32>> {
    Ok(target.to_physical(snapshot.window.scale_factor()?))
}

#[derive(Debug, PartialEq, Eq)]
enum PositionSettlement {
    AdoptProgrammatic(PhysicalPosition<i32>),
    ExternalMove,
    Unchanged,
}

fn positions_within_rounding_tolerance(
    requested: PhysicalPosition<i32>,
    observed: PhysicalPosition<i32>,
) -> bool {
    (requested.x - observed.x)
        .abs()
        .max((requested.y - observed.y).abs())
        <= NATIVE_POSITION_ROUNDING_TOLERANCE
}

fn position_settlement(
    programmatic_positions: &mut HashMap<String, PhysicalPosition<i32>>,
    id: &str,
    observed: PhysicalPosition<i32>,
    durable: LogicalPosition<i32>,
    scale: f64,
) -> PositionSettlement {
    if let Some(requested) = programmatic_positions.get(id).copied() {
        if positions_within_rounding_tolerance(requested, observed) {
            programmatic_positions.remove(id);
            PositionSettlement::AdoptProgrammatic(observed)
        } else {
            PositionSettlement::ExternalMove
        }
    } else if observed.to_logical::<i32>(scale) != durable {
        PositionSettlement::ExternalMove
    } else {
        PositionSettlement::Unchanged
    }
}

fn active_group_members(
    app: &AppHandle,
    repository: &NoteRepository,
    group: &StoredGroup,
    excluded: &HashSet<StoredGroupMember>,
) -> anyhow::Result<Vec<StoredGroupMember>> {
    let active_notes: HashSet<_> = repository
        .active()?
        .into_iter()
        .map(|note| note.id)
        .collect();
    let timer_repository = app.state::<TimerRepository>();
    let active_timers: HashSet<_> = if timer_repository.is_available() {
        timer_repository
            .all()?
            .into_iter()
            .filter(|timer| {
                app.get_webview_window(&format!("timer_{}", timer.id))
                    .is_some()
            })
            .map(|timer| timer.id)
            .collect()
    } else {
        HashSet::new()
    };
    Ok(group
        .members
        .iter()
        .filter(|member| {
            let active = match member.kind {
                GroupMemberKind::Note => active_notes.contains(&member.id),
                GroupMemberKind::Timer => active_timers.contains(&member.id),
            };
            active && !excluded.contains(*member)
        })
        .cloned()
        .collect())
}

fn layout_for_members_at_origin(
    app: &AppHandle,
    members: &[StoredGroupMember],
    origin_override: Option<LogicalPosition<i32>>,
) -> anyhow::Result<GroupLayout> {
    let snapshots = snapshots_for_members(app, members)?;
    let first = snapshots.first().context("Group had no active members")?;
    let origin = origin_override.unwrap_or(
        first
            .position
            .to_logical::<i32>(first.window.scale_factor()?),
    );
    let heights = snapshots
        .iter()
        .map(|snapshot| {
            stored_surface(app, &snapshot.member).map(|surface| surface.durable_height())
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let targets = arranged_positions(origin, &heights)?;
    Ok(GroupLayout { snapshots, targets })
}

fn restore_snapshots(
    snapshots: &[WindowSnapshot],
    geometries: &GeometryIndex,
    runtime: &mut GroupRuntimeState,
) {
    for snapshot in snapshots {
        if let Err(error) = snapshot.window.set_size(snapshot.size) {
            log::error!(
                "Could not restore window {:?} size: {error}",
                snapshot.member
            );
        }
        let position_restored = if let Err(error) = snapshot.window.set_position(snapshot.position)
        {
            log::error!(
                "Could not restore window {:?} position: {error}",
                snapshot.member
            );
            false
        } else {
            true
        };
        let _ = geometries.insert(
            snapshot.member.id.clone(),
            NoteGeometry {
                position: snapshot.position,
                size: snapshot.size,
            },
        );
        if position_restored {
            runtime.record_programmatic_position(snapshot.member.runtime_key(), snapshot.position);
        }
    }
}

fn apply_layout(
    layout: &GroupLayout,
    geometries: &GeometryIndex,
    runtime: &mut GroupRuntimeState,
) -> anyhow::Result<()> {
    for (index, (snapshot, target)) in layout.snapshots.iter().zip(&layout.targets).enumerate() {
        let current = snapshot
            .position
            .to_logical::<i32>(snapshot.window.scale_factor()?);
        let requested = requested_physical_position(snapshot, *target)?;
        if current != *target {
            if let Err(error) = snapshot.window.set_position(*target) {
                restore_snapshots(&layout.snapshots[..index], geometries, runtime);
                return Err(error).with_context(|| {
                    format!("Could not position group member {:?}", snapshot.member)
                });
            }
        }
        if let Err(error) = geometries.set_position(&snapshot.member.id, requested) {
            restore_snapshots(&layout.snapshots[..=index], geometries, runtime);
            return Err(error)
                .with_context(|| format!("Could not cache group member {:?}", snapshot.member));
        }
        if current != *target {
            runtime.record_programmatic_position(snapshot.member.runtime_key(), requested);
        }
    }
    Ok(())
}

fn layout_surface_replacements(
    app: &AppHandle,
    layout: &GroupLayout,
) -> anyhow::Result<Vec<StoredSurface>> {
    layout
        .snapshots
        .iter()
        .zip(&layout.targets)
        .map(|(snapshot, target)| {
            let mut surface = stored_surface(app, &snapshot.member)?;
            surface.set_position(target.x, target.y);
            Ok(surface)
        })
        .collect()
}

pub fn link_windows_on_this_side_below(
    app: &AppHandle,
    parent: &WebviewWindow,
) -> anyhow::Result<()> {
    let runtime_state = app.state::<GroupRuntime>();
    let mut runtime = runtime_state.lock()?;
    open_missing_active_notes(app)?;
    let repository = app.state::<NoteRepository>();
    let geometries = app.state::<GeometryIndex>();
    let parent_member = StoredGroupMember::from_window_label(parent.label())?;
    let groups = repository.all_groups()?;
    let existing_group = groups
        .iter()
        .find(|group| group.members.contains(&parent_member))
        .cloned();
    let mut active_members: Vec<_> = repository
        .active()?
        .into_iter()
        .map(|note| StoredGroupMember::note(note.id))
        .collect();
    let timer_repository = app.state::<TimerRepository>();
    if timer_repository.is_available() {
        active_members.extend(
            timer_repository
                .all()?
                .into_iter()
                .filter(|timer| {
                    app.get_webview_window(&format!("timer_{}", timer.id))
                        .is_some()
                })
                .map(|timer| StoredGroupMember::timer(timer.id)),
        );
    }
    let globally_active: HashSet<_> = active_members.iter().cloned().collect();
    active_members.retain(|member| member != &parent_member);
    let monitor = parent
        .current_monitor()?
        .context("Selected parent did not have a current monitor")?;
    let monitor_rect = WindowRect::from_physical(*monitor.position(), *monitor.size());
    let work_area =
        WindowRect::from_physical(monitor.work_area().position, monitor.work_area().size);
    let eligible_active = members_on_anchor_monitor_side(
        &parent_member,
        &active_members,
        &geometries,
        monitor_rect,
        work_area,
    )?;
    let mut selected_members: HashSet<_> = eligible_active.into_iter().collect();
    selected_members.insert(parent_member.clone());
    let absorbed_groups = groups
        .into_iter()
        .filter(|group| {
            group
                .members
                .iter()
                .any(|member| selected_members.contains(member))
        })
        .collect::<Vec<_>>();
    let mut other_members = selected_members;
    for group in &absorbed_groups {
        other_members.extend(
            group
                .members
                .iter()
                .filter(|member| globally_active.contains(*member))
                .cloned(),
        );
    }
    other_members.remove(&parent_member);
    let mut order = vec![parent_member.clone()];
    order.extend(visual_order(&other_members, &geometries)?);
    if order.len() < 2 {
        bail!("No other active windows are available to link on this monitor side");
    }

    let parent_geometry = geometries.get(&parent_member.id)?;
    let parent_origin = parent_geometry
        .position
        .to_logical::<i32>(parent.scale_factor()?);
    let layout = layout_for_members_at_origin(app, &order, Some(parent_origin))?;
    apply_layout(&layout, &geometries, &mut runtime)?;
    let replacements = layout_surface_replacements(app, &layout)?;
    let originals = match persist_surface_changes(app, &replacements) {
        Ok(originals) => originals,
        Err(error) => {
            restore_snapshots(&layout.snapshots, &geometries, &mut runtime);
            return Err(error.context("Could not persist linked window positions"));
        }
    };

    let group_id = existing_group
        .as_ref()
        .map(|group| group.id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let absorbed_group_ids = absorbed_groups
        .iter()
        .map(|group| group.id.clone())
        .collect::<HashSet<_>>();
    let durable_order = durable_relink_order(&parent_member, &order, &absorbed_groups);
    let persist = repository.mutate(|store| {
        persist_relinked_group(store, &absorbed_group_ids, &group_id, &durable_order);
        Ok(())
    });
    if let Err(error) = persist {
        restore_surface_changes(app, &originals);
        restore_snapshots(&layout.snapshots, &geometries, &mut runtime);
        return Err(error.context("Could not persist linked group"));
    }
    Ok(())
}

pub fn link_windows_on_this_side_below_focused(app: &AppHandle) -> anyhow::Result<()> {
    let parent = get_focused_window(app).context("No note or timer is currently focused")?;
    link_windows_on_this_side_below(app, &parent)
}

pub fn unlink_group_for_focused(app: &AppHandle) -> anyhow::Result<()> {
    let runtime_state = app.state::<GroupRuntime>();
    let _runtime = runtime_state.lock()?;
    let window = get_focused_window(app).context("No note or timer is currently focused")?;
    let member = StoredGroupMember::from_window_label(window.label())?;
    let repository = app.state::<NoteRepository>();
    let group = repository
        .group_for_member(&member)?
        .context("The focused window is not in a linked group")?;
    repository.mutate(|store| {
        store.groups.remove(&group.id);
        Ok(())
    })
}

fn restore_stored_geometry(
    window: &WebviewWindow,
    surface: &StoredSurface,
    geometries: &GeometryIndex,
    runtime: &mut GroupRuntimeState,
) {
    let member = surface.member();
    let result = (|| -> anyhow::Result<()> {
        let requested_size = LogicalSize::new(surface.width(), surface.durable_height());
        let requested_position = LogicalPosition::new(surface.x(), surface.y());
        window.set_size(requested_size)?;
        window.set_position(requested_position)?;
        let scale = window.scale_factor()?;
        let geometry = NoteGeometry {
            position: requested_position.to_physical(scale),
            size: requested_size.to_physical(scale),
        };
        geometries.insert(member.id.clone(), geometry)?;
        runtime.record_programmatic_position(member.runtime_key(), geometry.position);
        Ok(())
    })();
    if let Err(error) = result {
        log::error!("Could not restore window {:?} geometry: {error:#}", member);
    }
}

fn persist_group_detachment(
    store: &mut crate::save_load::NoteStore,
    group_id: &str,
    member: &StoredGroupMember,
) -> anyhow::Result<()> {
    let stored_group = store
        .groups
        .get_mut(group_id)
        .with_context(|| format!("Cannot detach from missing group {group_id}"))?;
    stored_group.members.retain(|candidate| candidate != member);
    if stored_group.members.len() < 2 {
        store.groups.remove(group_id);
    }
    Ok(())
}

fn detach_member(
    window: &WebviewWindow,
    group: &StoredGroup,
    geometry: NoteGeometry,
    runtime: &mut GroupRuntimeState,
) -> anyhow::Result<()> {
    let app = window.app_handle();
    let member = StoredGroupMember::from_window_label(window.label())?;
    let repository = app.state::<NoteRepository>();
    let geometries = app.state::<GeometryIndex>();
    let previous = stored_surface(app, &member)?;
    let scale = window.scale_factor()?;
    let position = geometry.position.to_logical::<i32>(scale);
    let size = geometry.size.to_logical::<u32>(scale);
    let mut replacement = previous.clone();
    replacement.set_position(position.x, position.y);
    if let StoredSurface::Note(note) = &mut replacement {
        if !note.collapsed {
            note.expanded_width = size.width.max(150);
            note.expanded_height = size.height.max(80);
        }
    }
    let originals = persist_surface_changes(app, &[replacement])?;
    if let Err(error) =
        repository.mutate(|store| persist_group_detachment(store, &group.id, &member))
    {
        restore_surface_changes(app, &originals);
        restore_stored_geometry(window, &previous, &geometries, runtime);
        return Err(error.context("Could not persist group detachment"));
    }
    Ok(())
}

fn drag_exceeds_threshold(start: LogicalPosition<i32>, end: LogicalPosition<i32>) -> bool {
    (end.x - start.x).abs().max((end.y - start.y).abs()) > DRAG_DETACH_THRESHOLD
}

pub fn run_window_drag<F>(window: &WebviewWindow, drag: F) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<()>,
{
    let app = window.app_handle();
    let runtime_state = app.state::<GroupRuntime>();
    let mut runtime = runtime_state.lock()?;
    let member = StoredGroupMember::from_window_label(window.label())?;
    let key = member.runtime_key();
    let origin = app.state::<GeometryIndex>().get(&member.id)?.position;
    runtime.begin_user_drag(key.clone(), origin);
    if let Err(error) = drag() {
        runtime.cancel_user_drag(&key);
        return Err(error);
    }
    runtime.complete_user_drag(&key)
}

fn resize_group_member(
    window: &WebviewWindow,
    group: &StoredGroup,
    old_surface: &StoredSurface,
    target_size: LogicalSize<u32>,
    collapsed: Option<bool>,
    runtime: &mut GroupRuntimeState,
) -> anyhow::Result<()> {
    let app = window.app_handle();
    let member = StoredGroupMember::from_window_label(window.label())?;
    let repository = app.state::<NoteRepository>();
    let geometries = app.state::<GeometryIndex>();
    let active_members = active_group_members(app, &repository, group, &HashSet::new())?;
    let index = active_members
        .iter()
        .position(|candidate| candidate == &member)
        .context("Group did not contain the resized active window")?;
    let selected_geometry = geometries.get(&member.id)?;
    let scale = window.scale_factor()?;
    let previous_logical_size = LogicalSize::new(old_surface.width(), old_surface.durable_height());
    let selected_snapshot = WindowSnapshot {
        member: member.clone(),
        window: window.clone(),
        position: selected_geometry.position,
        size: previous_logical_size.to_physical(scale),
    };
    let later_snapshots = snapshots_for_members(app, &active_members[index + 1..])?;
    let later_heights = later_snapshots
        .iter()
        .map(|snapshot| {
            stored_surface(app, &snapshot.member).map(|surface| surface.durable_height())
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let targets = positions_after_changed_note(
        LogicalPosition::new(old_surface.x(), old_surface.y()),
        target_size.height,
        &later_heights,
    )?;

    let native = (|| -> anyhow::Result<()> {
        if collapsed == Some(true) && window.is_maximized()? {
            window.unmaximize()?;
        }
        if let Some(collapsed) = collapsed {
            window.set_resizable(member.kind == GroupMemberKind::Note && !collapsed)?;
        }
        window.set_size(target_size)?;
        geometries.set_size(&member.id, target_size.to_physical(scale))?;
        for (snapshot, target) in later_snapshots.iter().zip(&targets) {
            snapshot.window.set_position(*target)?;
            let requested = requested_physical_position(snapshot, *target)?;
            geometries.set_position(&snapshot.member.id, requested)?;
            runtime.record_programmatic_position(snapshot.member.runtime_key(), requested);
        }
        Ok(())
    })();
    if let Err(error) = native {
        restore_snapshots(&later_snapshots, &geometries, runtime);
        restore_snapshots(
            std::slice::from_ref(&selected_snapshot),
            &geometries,
            runtime,
        );
        let _ =
            window.set_resizable(member.kind == GroupMemberKind::Note && !old_surface.collapsed());
        return Err(error.context("Could not resize linked group member"));
    }

    let mut replacements = Vec::with_capacity(later_snapshots.len() + 1);
    let mut selected = old_surface.clone();
    match (&mut selected, collapsed) {
        (StoredSurface::Note(note), None) if !note.collapsed => {
            note.expanded_width = target_size.width.max(150);
            note.expanded_height = target_size.height.max(80);
        }
        (surface, Some(collapsed)) => {
            surface.set_collapsed_and_size(collapsed, previous_logical_size)
        }
        _ => {}
    }
    replacements.push(selected);
    for (snapshot, target) in later_snapshots.iter().zip(&targets) {
        let mut later = stored_surface(app, &snapshot.member)?;
        later.set_position(target.x, target.y);
        replacements.push(later);
    }
    if let Err(error) = persist_surface_changes(app, &replacements) {
        restore_snapshots(&later_snapshots, &geometries, runtime);
        restore_snapshots(
            std::slice::from_ref(&selected_snapshot),
            &geometries,
            runtime,
        );
        let _ =
            window.set_resizable(member.kind == GroupMemberKind::Note && !old_surface.collapsed());
        return Err(error.context("Could not persist linked group resize"));
    }
    Ok(())
}

fn set_group_member_collapsed(
    window: &WebviewWindow,
    collapsed: bool,
    runtime: &mut GroupRuntimeState,
) -> anyhow::Result<()> {
    let app = window.app_handle();
    let member = StoredGroupMember::from_window_label(window.label())?;
    let repository = app.state::<NoteRepository>();
    let current = stored_surface(app, &member)?;
    if current.collapsed() == collapsed {
        return Ok(());
    }
    let group = repository
        .group_for_member(&member)?
        .context("Window was no longer in a linked group")?;
    let current_size = app
        .state::<GeometryIndex>()
        .get(&member.id)?
        .size
        .to_logical::<u32>(window.scale_factor()?);
    let target = LogicalSize::new(
        current.width().max(current_size.width),
        if collapsed {
            COLLAPSED_HEIGHT
        } else {
            current.expanded_height()
        },
    );
    resize_group_member(window, &group, &current, target, Some(collapsed), runtime)?;
    if !collapsed {
        window.set_focus()?;
    }
    Ok(())
}

pub fn set_window_collapsed(window: &WebviewWindow, collapsed: bool) -> anyhow::Result<()> {
    let app = window.app_handle();
    let runtime_state = app.state::<GroupRuntime>();
    let mut runtime = runtime_state.lock()?;
    let member = StoredGroupMember::from_window_label(window.label())?;
    if app
        .state::<NoteRepository>()
        .group_for_member(&member)?
        .is_some()
    {
        set_group_member_collapsed(window, collapsed, &mut runtime)
    } else if member.kind == GroupMemberKind::Note {
        set_ungrouped_window_collapsed(window, collapsed)
    } else {
        set_ungrouped_timer_collapsed(window, collapsed)
    }
}

pub fn resize_note_height(window: &WebviewWindow, height: u32) -> anyhow::Result<()> {
    let app = window.app_handle();
    let runtime_state = app.state::<GroupRuntime>();
    let mut runtime = runtime_state.lock()?;
    let id = note_id_from_label(window.label())?;
    let repository = app.state::<NoteRepository>();
    let current = repository.get(id)?;
    if current.collapsed {
        return Ok(());
    }
    let geometry = app.state::<GeometryIndex>().get(id)?;
    let size = geometry.size.to_logical::<u32>(window.scale_factor()?);
    let target = LogicalSize::new(size.width.max(150), height.max(80));
    if let Some(group) = repository.group_for_note(id)? {
        resize_group_member(
            window,
            &group,
            &StoredSurface::Note(current),
            target,
            None,
            &mut runtime,
        )
    } else {
        let snapshot = WindowSnapshot {
            member: StoredGroupMember::note(id),
            window: window.clone(),
            position: geometry.position,
            size: geometry.size,
        };
        if let Err(error) = window.set_size(target) {
            return Err(error.into());
        }
        let physical_size = window.outer_size()?;
        app.state::<GeometryIndex>().set_size(id, physical_size)?;
        let logical = physical_size.to_logical::<u32>(window.scale_factor()?);
        if let Err(error) = repository.update(id, |note| {
            note.expanded_width = logical.width.max(150);
            note.expanded_height = logical.height.max(80);
            Ok(())
        }) {
            restore_snapshots(&[snapshot], &app.state::<GeometryIndex>(), &mut runtime);
            return Err(error.context("Could not persist note height"));
        }
        Ok(())
    }
}

pub fn settle_window_geometry(window: &WebviewWindow) -> anyhow::Result<()> {
    let app = window.app_handle();
    let runtime_state = app.state::<GroupRuntime>();
    let mut runtime = runtime_state.lock()?;
    let member = StoredGroupMember::from_window_label(window.label())?;
    let key = member.runtime_key();
    if runtime.drag_origins.contains_key(&key) {
        return Ok(());
    }
    let geometries = app.state::<GeometryIndex>();
    let geometry = geometries.get(&member.id)?;
    let scale = window.scale_factor()?;
    let position = geometry.position.to_logical::<i32>(scale);
    let size = geometry.size.to_logical::<u32>(scale);
    let repository = app.state::<NoteRepository>();
    let current = stored_surface(app, &member)?;
    let group = repository.group_for_member(&member)?;
    if let Some(origin) = runtime.take_completed_drag(&key) {
        let start = origin.to_logical::<i32>(scale);
        let end = geometry.position.to_logical::<i32>(scale);
        if let Some(group) = &group {
            if drag_exceeds_threshold(start, end) {
                return detach_member(window, group, geometry, &mut runtime);
            }
            if geometry.position != origin {
                window.set_position(origin)?;
                geometries.set_position(&member.id, origin)?;
                runtime.record_programmatic_position(key.clone(), origin);
            }
        } else {
            let mut replacement = current.clone();
            replacement.set_position(position.x, position.y);
            if let StoredSurface::Note(note) = &mut replacement {
                if !note.collapsed {
                    note.expanded_width = size.width.max(150);
                    note.expanded_height = size.height.max(80);
                }
            }
            persist_surface_changes(app, &[replacement])?;
            return Ok(());
        }
    } else {
        match position_settlement(
            &mut runtime.programmatic_positions,
            &key,
            geometry.position,
            LogicalPosition::new(current.x(), current.y()),
            scale,
        ) {
            PositionSettlement::AdoptProgrammatic(observed) => {
                if position.x != current.x() || position.y != current.y() {
                    let mut replacement = current.clone();
                    replacement.set_position(position.x, position.y);
                    persist_surface_changes(app, &[replacement])?;
                }
                geometries.set_position(&member.id, observed)?;
            }
            PositionSettlement::ExternalMove => {
                if current.pinned() {
                    window.set_position(LogicalPosition::new(current.x(), current.y()))?;
                    let requested =
                        LogicalPosition::new(current.x(), current.y()).to_physical(scale);
                    geometries.set_position(&member.id, requested)?;
                    runtime.record_programmatic_position(key.clone(), requested);
                }
            }
            PositionSettlement::Unchanged => {}
        }
    }
    if group.is_none() {
        if let StoredSurface::Note(note) = current {
            if !note.collapsed
                && (size.width != note.expanded_width || size.height != note.expanded_height)
            {
                let mut replacement = StoredSurface::Note(note);
                if let StoredSurface::Note(note) = &mut replacement {
                    note.expanded_width = size.width.max(150);
                    note.expanded_height = size.height.max(80);
                }
                persist_surface_changes(app, &[replacement])?;
            }
        }
        return Ok(());
    }
    let group = group.context("Window was no longer in a linked group")?;
    if current.collapsed()
        || (size.width == current.width() && size.height == current.expanded_height())
    {
        return Ok(());
    }
    resize_group_member(
        window,
        &group,
        &current,
        LogicalSize::new(size.width.max(150), size.height.max(80)),
        None,
        &mut runtime,
    )
}

fn close_group_member(
    window: &WebviewWindow,
    runtime: &mut GroupRuntimeState,
) -> anyhow::Result<()> {
    let app = window.app_handle();
    let id = note_id_from_label(window.label())?.to_string();
    let repository = app.state::<NoteRepository>();
    let previous = repository.get(&id)?;
    let member = StoredGroupMember::note(&id);
    let group = repository
        .group_for_note(&id)?
        .context("Note was no longer in a linked group")?;
    let active_before = active_group_members(app, &repository, &group, &HashSet::new())?;
    let removed_top = active_before
        .first()
        .is_some_and(|candidate| candidate == &member);
    let remaining =
        active_group_members(app, &repository, &group, &HashSet::from([member.clone()]))?;
    let layout = if remaining.is_empty() {
        None
    } else {
        Some(layout_for_members_at_origin(
            app,
            &remaining,
            removed_top.then_some(LogicalPosition::new(previous.x, previous.y)),
        )?)
    };
    if let Some(layout) = &layout {
        apply_layout(layout, &app.state::<GeometryIndex>(), runtime)?;
    }
    let replacements = layout
        .as_ref()
        .map(|layout| layout_surface_replacements(app, layout))
        .transpose()?
        .unwrap_or_default();
    let originals = match persist_surface_changes(app, &replacements) {
        Ok(originals) => originals,
        Err(error) => {
            if let Some(layout) = &layout {
                restore_snapshots(&layout.snapshots, &app.state::<GeometryIndex>(), runtime);
            }
            return Err(error.context("Could not persist linked group compaction"));
        }
    };
    let closed_at = current_time_millis()?;
    if let Err(error) = repository.update(&id, |note| {
        note.closed_at = Some(closed_at);
        Ok(())
    }) {
        restore_surface_changes(app, &originals);
        if let Some(layout) = &layout {
            restore_snapshots(&layout.snapshots, &app.state::<GeometryIndex>(), runtime);
        }
        return Err(error.context("Could not archive linked group member"));
    }
    if let Err(close_error) = window.close() {
        let rollback = repository.update(&id, |note| {
            *note = previous.clone();
            Ok(())
        });
        restore_surface_changes(app, &originals);
        if let Some(layout) = &layout {
            restore_snapshots(&layout.snapshots, &app.state::<GeometryIndex>(), runtime);
        }
        rollback.with_context(|| {
            format!("Could not roll back failed window close after: {close_error}")
        })?;
        return Err(close_error.into());
    }
    let key = member.runtime_key();
    runtime.drag_origins.remove(&key);
    runtime.completed_drag_origins.remove(&key);
    runtime.programmatic_positions.remove(&key);
    Ok(())
}

fn close_grouped_timer(
    window: &WebviewWindow,
    runtime: &mut GroupRuntimeState,
) -> anyhow::Result<()> {
    let app = window.app_handle();
    let member = StoredGroupMember::from_window_label(window.label())?;
    let repository = app.state::<NoteRepository>();
    let group = repository
        .group_for_member(&member)?
        .context("Timer was no longer in a linked group")?;
    let previous_group = group.clone();
    let previous_timer = stored_surface(app, &member)?;
    let active_before = active_group_members(app, &repository, &group, &HashSet::new())?;
    let removed_top = active_before
        .first()
        .is_some_and(|candidate| candidate == &member);
    let remaining =
        active_group_members(app, &repository, &group, &HashSet::from([member.clone()]))?;
    let layout = if remaining.is_empty() {
        None
    } else {
        Some(layout_for_members_at_origin(
            app,
            &remaining,
            removed_top.then_some(LogicalPosition::new(previous_timer.x(), previous_timer.y())),
        )?)
    };
    if let Some(layout) = &layout {
        apply_layout(layout, &app.state::<GeometryIndex>(), runtime)?;
    }
    let replacements = layout
        .as_ref()
        .map(|layout| layout_surface_replacements(app, layout))
        .transpose()?
        .unwrap_or_default();
    let originals = persist_surface_changes(app, &replacements)?;
    if let Err(error) =
        repository.mutate(|store| persist_group_detachment(store, &group.id, &member))
    {
        restore_surface_changes(app, &originals);
        if let Some(layout) = &layout {
            restore_snapshots(&layout.snapshots, &app.state::<GeometryIndex>(), runtime);
        }
        return Err(error.context("Could not remove timer from linked group"));
    }
    let removed_timer = match remove_timer_for_close(window) {
        Ok(timer) => timer,
        Err(error) => {
            let _ = repository.mutate(|store| {
                store
                    .groups
                    .insert(previous_group.id.clone(), previous_group.clone());
                Ok(())
            });
            restore_surface_changes(app, &originals);
            if let Some(layout) = &layout {
                restore_snapshots(&layout.snapshots, &app.state::<GeometryIndex>(), runtime);
            }
            return Err(error.context("Could not delete linked timer"));
        }
    };
    if let Err(close_error) = window.close() {
        restore_timer_after_failed_close(window, removed_timer)?;
        repository.mutate(|store| {
            store
                .groups
                .insert(previous_group.id.clone(), previous_group.clone());
            Ok(())
        })?;
        restore_surface_changes(app, &originals);
        if let Some(layout) = &layout {
            restore_snapshots(&layout.snapshots, &app.state::<GeometryIndex>(), runtime);
        }
        return Err(close_error.into());
    }
    let key = member.runtime_key();
    runtime.drag_origins.remove(&key);
    runtime.completed_drag_origins.remove(&key);
    runtime.programmatic_positions.remove(&key);
    Ok(())
}

pub fn close_window(window: &WebviewWindow) -> anyhow::Result<()> {
    let app = window.app_handle();
    let runtime_state = app.state::<GroupRuntime>();
    let mut runtime = runtime_state.lock()?;
    let member = StoredGroupMember::from_window_label(window.label())?;
    let group = app.state::<NoteRepository>().group_for_member(&member)?;
    match (member.kind, group.is_some()) {
        (GroupMemberKind::Note, true) => close_group_member(window, &mut runtime),
        (GroupMemberKind::Note, false) => close_ungrouped_window_and_archive(window),
        (GroupMemberKind::Timer, true) => close_grouped_timer(window, &mut runtime),
        (GroupMemberKind::Timer, false) => close_timer(window.clone()).map_err(anyhow::Error::msg),
    }
}

fn restore_archived_note(
    app: &AppHandle,
    note: &StoredNote,
    runtime: &mut GroupRuntimeState,
) -> anyhow::Result<WebviewWindow> {
    let repository = app.state::<NoteRepository>();
    let window = open_sticky(app, note).context("Could not open archived note")?;
    let restore = (|| -> anyhow::Result<()> {
        let group = repository.group_for_note(&note.id)?;
        let layout = group
            .as_ref()
            .map(|group| {
                let mut active_notes: HashSet<_> = repository
                    .active()?
                    .into_iter()
                    .map(|active_note| active_note.id)
                    .collect();
                active_notes.insert(note.id.clone());
                let timers = app.state::<TimerRepository>();
                let active_timers: HashSet<_> = if timers.is_available() {
                    timers
                        .all()?
                        .into_iter()
                        .filter(|timer| {
                            app.get_webview_window(&format!("timer_{}", timer.id))
                                .is_some()
                        })
                        .map(|timer| timer.id)
                        .collect()
                } else {
                    HashSet::new()
                };
                let members: Vec<_> = group
                    .members
                    .iter()
                    .filter(|member| match member.kind {
                        GroupMemberKind::Note => active_notes.contains(&member.id),
                        GroupMemberKind::Timer => active_timers.contains(&member.id),
                    })
                    .cloned()
                    .collect();
                layout_for_members_at_origin(app, &members, None).map(Some)
            })
            .transpose()?
            .flatten();
        if let Some(layout) = &layout {
            apply_layout(layout, &app.state::<GeometryIndex>(), runtime)?;
        }
        let replacements = layout
            .as_ref()
            .map(|layout| layout_surface_replacements(app, layout))
            .transpose()?
            .unwrap_or_default();
        let originals = persist_surface_changes(app, &replacements)?;
        let persist = repository.update(&note.id, |stored| {
            stored.closed_at = None;
            Ok(())
        });
        if let Err(error) = persist {
            restore_surface_changes(app, &originals);
            if let Some(layout) = &layout {
                restore_snapshots(&layout.snapshots, &app.state::<GeometryIndex>(), runtime);
            }
            return Err(error.context("Could not persist restored group member"));
        }
        Ok(())
    })();
    if let Err(error) = restore {
        let _ = window.close();
        return Err(error);
    }
    Ok(window)
}

pub fn restore_last_closed(app: &AppHandle) -> anyhow::Result<()> {
    let runtime_state = app.state::<GroupRuntime>();
    let mut runtime = runtime_state.lock()?;
    let note = app
        .state::<NoteRepository>()
        .last_closed()?
        .context("No recently closed note")?;
    restore_archived_note(app, &note, &mut runtime)?
        .set_focus()
        .context("Could not focus restored note")
}

pub fn restore_all_notes(app: &AppHandle) -> anyhow::Result<()> {
    let runtime_state = app.state::<GroupRuntime>();
    let mut runtime = runtime_state.lock()?;
    let repository = app.state::<NoteRepository>();
    let mut archived = repository.archived()?;
    archived.sort_by_key(|note| note.closed_at);
    for note in archived {
        restore_archived_note(app, &note, &mut runtime)?;
    }
    open_missing_active_notes(app)?;
    let windows = sorted_windows(app);
    if windows.is_empty() {
        bail!("No notes to restore");
    }
    for window in &windows {
        window.show()?;
        if window.is_minimized()? {
            window.unminimize()?;
        }
    }
    windows[0]
        .set_focus()
        .context("Could not focus a restored note")
}

fn reset_positions_in_work_area(
    work_area: WindowRect,
    count: usize,
    preferred_step: i32,
    header_height: i32,
) -> anyhow::Result<Vec<PhysicalPosition<i32>>> {
    if count == 0 {
        return Ok(Vec::new());
    }
    let margin = i64::from(RESET_MARGIN.max(0));
    let header_height = i64::from(header_height.max(1));
    let x = work_area.x + margin.min(work_area.width.saturating_sub(1).max(0));
    let top = work_area.y + margin.min(work_area.height.saturating_sub(1).max(0));
    let bottom = (work_area.bottom() - header_height - margin).max(top);
    let available = bottom - top;
    let step = if count == 1 {
        0
    } else {
        i64::from(preferred_step.max(0)).min(available / (count as i64 - 1))
    };
    (0..count)
        .map(|index| {
            Ok(PhysicalPosition::new(
                i32::try_from(x).context("Reset x-position exceeded platform limits")?,
                i32::try_from(top + index as i64 * step)
                    .context("Reset y-position exceeded platform limits")?,
            ))
        })
        .collect()
}

pub fn reset_note_positions(app: &AppHandle) -> anyhow::Result<()> {
    let runtime_state = app.state::<GroupRuntime>();
    let mut runtime = runtime_state.lock()?;
    open_missing_active_notes(app)?;
    let geometries = app.state::<GeometryIndex>();
    let snapshots = sorted_windows(app)
        .into_iter()
        .map(|window| {
            let id = note_id_from_label(window.label())?.to_string();
            let geometry = geometries.get(&id)?;
            Ok(WindowSnapshot {
                member: StoredGroupMember::note(id),
                window,
                position: geometry.position,
                size: geometry.size,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    if snapshots.is_empty() {
        return Ok(());
    }
    let monitor = app
        .primary_monitor()?
        .context("No primary monitor available for resetting note positions")?;
    let scale = monitor.scale_factor();
    let work_area =
        WindowRect::from_physical(monitor.work_area().position, monitor.work_area().size);
    let step = (f64::from(COLLAPSED_HEIGHT + GROUP_GAP) * scale).round() as i32;
    let header_height = (f64::from(COLLAPSED_HEIGHT) * scale).round() as i32;
    let targets = reset_positions_in_work_area(work_area, snapshots.len(), step, header_height)?;
    for snapshot in &snapshots {
        snapshot.window.show()?;
        if snapshot.window.is_minimized()? {
            snapshot.window.unminimize()?;
        }
    }
    for (index, (snapshot, target)) in snapshots.iter().zip(&targets).enumerate() {
        if let Err(error) = snapshot.window.set_position(*target) {
            restore_snapshots(&snapshots[..index], &geometries, &mut runtime);
            return Err(error.into());
        }
        geometries.set_position(&snapshot.member.id, *target)?;
        runtime.record_programmatic_position(snapshot.member.runtime_key(), *target);
    }
    let positions = snapshots
        .iter()
        .zip(&targets)
        .map(|(snapshot, target)| {
            let logical = target.to_logical::<i32>(snapshot.window.scale_factor()?);
            Ok((snapshot.member.id.clone(), logical.x, logical.y))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let persist = app.state::<NoteRepository>().mutate(|store| {
        store.groups.clear();
        for (id, x, y) in &positions {
            let note = store
                .notes
                .get_mut(id)
                .with_context(|| format!("Cannot reset missing note {id}"))?;
            note.x = *x;
            note.y = *y;
        }
        Ok(())
    });
    if let Err(error) = persist {
        restore_snapshots(&snapshots, &geometries, &mut runtime);
        return Err(error.context("Could not persist reset note positions"));
    }
    snapshots[0]
        .window
        .set_focus()
        .context("Could not focus reset notes")
}

pub fn restore_group_layouts(app: &AppHandle) -> anyhow::Result<()> {
    let runtime_state = app.state::<GroupRuntime>();
    let mut runtime = runtime_state.lock()?;
    let repository = app.state::<NoteRepository>();
    let geometries = app.state::<GeometryIndex>();
    let mut layouts = Vec::new();
    for group in repository.all_groups()? {
        let members = active_group_members(app, &repository, &group, &HashSet::new())?;
        if !members.is_empty() {
            layouts.push(layout_for_members_at_origin(app, &members, None)?);
        }
    }
    if layouts.is_empty() {
        return Ok(());
    }
    for (index, layout) in layouts.iter().enumerate() {
        if let Err(error) = apply_layout(layout, &geometries, &mut runtime) {
            for applied in &layouts[..index] {
                restore_snapshots(&applied.snapshots, &geometries, &mut runtime);
            }
            return Err(error.context("Could not restore linked group layouts"));
        }
    }
    let replacements = layouts
        .iter()
        .map(|layout| layout_surface_replacements(app, layout))
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if let Err(error) = persist_surface_changes(app, &replacements) {
        for layout in &layouts {
            restore_snapshots(&layout.snapshots, &geometries, &mut runtime);
        }
        return Err(error.context("Could not persist restored linked group layouts"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(x: i32, y: i32, width: u32, height: u32) -> NoteGeometry {
        NoteGeometry {
            position: PhysicalPosition::new(x, y),
            size: PhysicalSize::new(width, height),
        }
    }

    #[test]
    fn one_click_linking_selects_typed_windows_only_on_the_parent_side() {
        let geometries = GeometryIndex::default();
        for (id, note_geometry) in [
            ("parent", geometry(100, 100, 280, 100)),
            ("top-right", geometry(300, 20, 220, 100)),
            ("bottom", geometry(200, 80, 360, 100)),
            ("top-left", geometry(100, 20, 180, 100)),
            ("other-side", geometry(700, 10, 300, 100)),
            ("other-monitor", geometry(1100, 0, 300, 100)),
            ("timer", geometry(160, 180, 300, 176)),
        ] {
            geometries.insert(id.into(), note_geometry).unwrap();
        }
        let monitor = WindowRect {
            x: 0,
            y: 0,
            width: 1000,
            height: 800,
        };
        let members = vec![
            StoredGroupMember::note("top-right"),
            StoredGroupMember::note("bottom"),
            StoredGroupMember::note("top-left"),
            StoredGroupMember::note("other-side"),
            StoredGroupMember::note("other-monitor"),
            StoredGroupMember::timer("timer"),
        ];

        assert_eq!(
            members_on_anchor_monitor_side(
                &StoredGroupMember::note("parent"),
                &members,
                &geometries,
                monitor,
                monitor,
            )
            .unwrap(),
            vec![
                StoredGroupMember::note("top-left"),
                StoredGroupMember::note("top-right"),
                StoredGroupMember::note("bottom"),
                StoredGroupMember::timer("timer"),
            ]
        );
    }

    #[test]
    fn relinking_from_an_independent_parent_absorbs_a_mixed_group_and_preserves_slots() {
        let parent = StoredGroupMember::note("parent");
        let dump = StoredGroupMember::note("dump");
        let archived = StoredGroupMember::note("archived");
        let timer = StoredGroupMember::timer("timer");
        let absorbed_group = StoredGroup {
            id: "lower".into(),
            members: vec![dump.clone(), archived.clone(), timer.clone()],
        };
        let durable_order = durable_relink_order(
            &parent,
            &[parent.clone(), dump.clone(), timer.clone()],
            std::slice::from_ref(&absorbed_group),
        );
        let mut store: crate::save_load::NoteStore = serde_json::from_value(serde_json::json!({
            "version": 4,
            "notes": {
                "parent": {
                    "id": "parent", "document": {"type": "doc", "content": []},
                    "color": "#fff9b1", "x": 20, "y": 20,
                    "expanded_height": 250, "expanded_width": 300,
                    "collapsed": false, "pinned": false, "font_size": 16
                },
                "dump": {
                    "id": "dump", "document": {"type": "doc", "content": []},
                    "color": "#fff9b1", "x": 20, "y": 282,
                    "expanded_height": 250, "expanded_width": 300,
                    "collapsed": false, "pinned": false, "font_size": 16
                },
                "archived": {
                    "id": "archived", "document": {"type": "doc", "content": []},
                    "color": "#fff9b1", "x": 20, "y": 544,
                    "expanded_height": 250, "expanded_width": 300,
                    "collapsed": true, "pinned": false, "font_size": 16,
                    "closed_at": 1
                },
                "other": {
                    "id": "other", "document": {"type": "doc", "content": []},
                    "color": "#fff9b1", "x": 900, "y": 20,
                    "expanded_height": 250, "expanded_width": 300,
                    "collapsed": false, "pinned": false, "font_size": 16
                }
            },
            "groups": {
                "lower": {"id": "lower", "members": [
                    {"kind": "note", "id": "dump"},
                    {"kind": "note", "id": "archived"},
                    {"kind": "timer", "id": "timer"}
                ]},
                "other-side": {"id": "other-side", "members": [
                    {"kind": "note", "id": "other"},
                    {"kind": "timer", "id": "other-timer"}
                ]}
            }
        }))
        .unwrap();

        persist_relinked_group(
            &mut store,
            &HashSet::from([absorbed_group.id]),
            "relinked",
            &durable_order,
        );

        assert_eq!(
            store.groups["relinked"].members,
            [parent, dump, archived, timer]
        );
        assert!(!store.groups.contains_key("lower"));
        assert_eq!(
            store.groups["other-side"].members,
            [
                StoredGroupMember::note("other"),
                StoredGroupMember::timer("other-timer"),
            ]
        );
    }

    #[test]
    fn restored_mixed_group_uses_durable_heights_and_twelve_pixel_gaps() {
        assert_eq!(
            arranged_positions(LogicalPosition::new(40, 20), &[24, 24, 24, 250, 250, 250],)
                .unwrap(),
            vec![
                LogicalPosition::new(40, 20),
                LogicalPosition::new(40, 56),
                LogicalPosition::new(40, 92),
                LogicalPosition::new(40, 128),
                LogicalPosition::new(40, 390),
                LogicalPosition::new(40, 652),
            ]
        );
    }

    #[test]
    fn height_transition_repositions_only_later_members_from_durable_heights() {
        assert_eq!(
            positions_after_changed_note(LogicalPosition::new(40, 282), 24, &[250, 24]).unwrap(),
            vec![LogicalPosition::new(40, 318), LogicalPosition::new(40, 580),]
        );
    }

    #[test]
    fn folding_a_timer_reflows_only_later_mixed_members() {
        assert_eq!(
            positions_after_changed_note(LogicalPosition::new(40, 282), 24, &[250, 176]).unwrap(),
            vec![LogicalPosition::new(40, 318), LogicalPosition::new(40, 580),]
        );
    }

    #[test]
    fn deleting_a_timer_compacts_remaining_windows_without_changing_order() {
        let members = [
            (StoredGroupMember::note("first"), 250),
            (StoredGroupMember::timer("deleted"), 176),
            (StoredGroupMember::note("last"), 250),
        ];
        let remaining_heights = members
            .iter()
            .filter(|(member, _)| member != &StoredGroupMember::timer("deleted"))
            .map(|(_, height)| *height)
            .collect::<Vec<_>>();

        assert_eq!(
            arranged_positions(LogicalPosition::new(40, 20), &remaining_heights).unwrap(),
            vec![LogicalPosition::new(40, 20), LogicalPosition::new(40, 282)]
        );
    }

    #[test]
    fn delayed_programmatic_move_adopts_native_rounding_instead_of_stale_coordinates() {
        let mut pending = HashMap::from([("note".to_string(), PhysicalPosition::new(100, 200))]);
        let rounded_native_position = PhysicalPosition::new(101, 201);

        assert_eq!(
            position_settlement(
                &mut pending,
                "note",
                rounded_native_position,
                LogicalPosition::new(20, 20),
                1.0,
            ),
            PositionSettlement::AdoptProgrammatic(rounded_native_position)
        );
        assert!(pending.is_empty());
    }

    #[test]
    fn external_move_does_not_consume_the_pending_programmatic_target() {
        let requested = PhysicalPosition::new(100, 200);
        let mut pending = HashMap::from([("note".to_string(), requested)]);

        assert_eq!(
            position_settlement(
                &mut pending,
                "note",
                PhysicalPosition::new(3439, 1354),
                LogicalPosition::new(20, 20),
                1.0,
            ),
            PositionSettlement::ExternalMove
        );
        assert_eq!(pending.get("note"), Some(&requested));
    }

    #[test]
    fn completed_user_drag_waits_for_settled_geometry_before_detachment() {
        let mut runtime = GroupRuntimeState::default();
        let origin = PhysicalPosition::new(100, 100);
        runtime.record_programmatic_position("note".into(), PhysicalPosition::new(90, 90));

        runtime.begin_user_drag("note".into(), origin);
        assert!(!runtime.programmatic_positions.contains_key("note"));
        runtime.complete_user_drag("note").unwrap();

        assert!(!runtime.drag_origins.contains_key("note"));
        let recorded_origin = runtime.take_completed_drag("note").unwrap();
        assert_eq!(recorded_origin, origin);
        assert!(drag_exceeds_threshold(
            recorded_origin.to_logical::<i32>(1.0),
            PhysicalPosition::new(250, 220).to_logical::<i32>(1.0),
        ));
    }

    #[test]
    fn drag_detachment_uses_a_strict_four_pixel_threshold() {
        let start = LogicalPosition::new(100, 100);
        assert!(!drag_exceeds_threshold(
            start,
            LogicalPosition::new(104, 96)
        ));
        assert!(drag_exceeds_threshold(
            start,
            LogicalPosition::new(105, 100)
        ));
    }

    #[test]
    fn dragging_a_typed_group_member_removes_only_that_membership() {
        let mut store: crate::save_load::NoteStore = serde_json::from_value(serde_json::json!({
            "version": 4,
            "notes": {
                "first": {
                    "id": "first", "document": {"type": "doc", "content": []},
                    "color": "#fff9b1", "x": 20, "y": 20,
                    "expanded_height": 250, "expanded_width": 300,
                    "collapsed": false, "pinned": false, "font_size": 16
                },
                "dragged": {
                    "id": "dragged", "document": {"type": "doc", "content": []},
                    "color": "#fff9b1", "x": 20, "y": 282,
                    "expanded_height": 250, "expanded_width": 300,
                    "collapsed": false, "pinned": false, "font_size": 16
                },
                "last": {
                    "id": "last", "document": {"type": "doc", "content": []},
                    "color": "#fff9b1", "x": 20, "y": 544,
                    "expanded_height": 250, "expanded_width": 300,
                    "collapsed": false, "pinned": false, "font_size": 16
                }
            },
            "groups": {
                "group": {"id": "group", "members": [
                    {"kind": "note", "id": "first"},
                    {"kind": "timer", "id": "dragged"},
                    {"kind": "note", "id": "last"}
                ]}
            }
        }))
        .unwrap();

        persist_group_detachment(&mut store, "group", &StoredGroupMember::timer("dragged"))
            .unwrap();

        assert_eq!((store.notes["first"].x, store.notes["first"].y), (20, 20));
        assert_eq!((store.notes["last"].x, store.notes["last"].y), (20, 544));
        assert_eq!(
            store.groups["group"].members,
            [
                StoredGroupMember::note("first"),
                StoredGroupMember::note("last"),
            ]
        );
    }

    #[test]
    fn reset_positions_keep_every_note_header_in_the_work_area() {
        let work_area = WindowRect {
            x: -1200,
            y: 24,
            width: 900,
            height: 100,
        };
        let positions = reset_positions_in_work_area(work_area, 4, 36, 24).unwrap();

        assert_eq!(positions[0], PhysicalPosition::new(-1180, 44));
        assert_eq!(positions[3], PhysicalPosition::new(-1180, 80));
        assert!(positions.iter().all(|position| {
            i64::from(position.y) >= work_area.y && i64::from(position.y) + 24 <= work_area.bottom()
        }));
    }
}

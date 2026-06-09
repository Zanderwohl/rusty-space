# System Graph

This document lists all Bevy systems in the application and their dependencies.

## Legend
- **Schedule**: When the system runs (Startup, Update, OnEnter, OnExit, EguiPrimaryContextPass)
- **Run Condition**: State or resource conditions that must be met
- **Depends On**: Systems that must run before this one (explicit `.after()` constraints)
- **Runs Before**: Systems that run after this one (explicit `.before()` constraints)

---

## Global Systems (Always Active)

### Startup

| System | Purpose | Dependencies |
|--------|---------|--------------|
| `common_setup` | Spawns camera with Freecam, PlanetariumCamera, Bloom, etc. | — |
| `load_catalogs` | Loads star catalog data from files | — |
| `spawn_starfield` | Creates starfield mesh and material | after `load_catalogs` |
| `initial_grab_on_flycam_spawn` | Sets up initial cursor grab state | — |

### Update (Unconditional)

| System | Purpose | Dependencies |
|--------|---------|--------------|
| `close_when_requested` | Handles window close, saves settings on change/close | — |
| `make_visible` | Makes window visible after frame 3 | — |
| `update_post_process_settings` | Updates bloom/tonemapping | run_if `resource_changed::<PostProcessSettings>` |
| `toggle_perf` | Toggles debug performance UI | — |

### FreeCam Systems (Update, always active)

| System | Purpose | Dependencies |
|--------|---------|--------------|
| `player_move` | WASD movement for free camera | — |
| `player_look` | Mouse look for free camera | — |
| `cursor_grab` | Handles cursor grab toggle | — |

---

## Splash Screen (AppState::Splash)

| Schedule | System | Purpose |
|----------|--------|---------|
| OnEnter | `splash_setup` | Creates splash screen UI |
| Update | `countdown` | Counts down and transitions to MainMenu |
| OnExit | `despawn_entities_with::<SplashScreen>` | Cleanup |

---

## Main Menu (AppState::MainMenu)

### EguiPrimaryContextPass

| System | Run Condition | Purpose |
|--------|---------------|---------|
| `main_menu` | MenuState::Home | Renders home menu buttons |
| `planetarium_menu` | MenuState::Planetarium | Renders save/load file picker |
| `settings_menu` | MenuState::Settings | Renders settings panel |

### Other

| Schedule | System | Purpose |
|----------|--------|---------|
| OnEnter(MenuState::Planetarium) | `load_planetarium_files` | Scans data/templates and data/saves |
| OnEnter(AppState::MainMenu) | `load_planetarium_files` | Re-scan if returning to planetarium menu |
| Update | `quit_system` | Handles quit request from UI |

---

## Planetarium Loading (AppState::PlanetariumLoading)

| Schedule | System | Purpose |
|----------|--------|---------|
| Update | `load_assets` | Loads universe file, spawns bodies |
| OnExit | `initial_trajectories` | Requests trajectory calculation for all bodies |

---

## Planetarium (AppState::Planetarium)

### System Sets
- **PlanetariumUISet**: Presentation and UI systems
- **PlanetariumSimulationSet**: Physics simulation systems

### Simulation Pipeline (PlanetariumSimulationSet)

| System | Purpose | Dependencies |
|--------|---------|--------------|
| `advance_time` | Advances SimTime based on delta and playback speed | — |

### Core Position Pipeline (PlanetariumUISet)

```
advance_time
     │
     ▼
calculate_body_positions ──► position_bodies ──► orient_bodies
                                   │
                                   ▼
                          [all presentation systems]
```

| System | Purpose | Depends On | Runs Before |
|--------|---------|------------|-------------|
| `calculate_body_positions` | Computes simulation positions via PhysicsGraph | after `advance_time` | — |
| `position_bodies` | Copies simulation positions to Bevy Transforms | after `calculate_body_positions` | — |
| `orient_bodies` | Applies body rotation/tilt | after `position_bodies` | — |

### Camera Systems (PlanetariumCameraPlugin)

| System | Purpose | Depends On | Runs Before |
|--------|---------|------------|-------------|
| `handle_gotos` | Processes GoTo messages, starts camera transitions | — | — |
| `run_goto` | Animates camera GoTo movement | after `calculate_body_positions` | before `position_bodies` |
| `revolve_around` | Updates camera revolve-around-body behavior | after `calculate_body_positions` | before `position_bodies` |
| `update_hover_target` | Raycasts for body/trajectory hover detection | — | — |
| `pick_body_on_click` | Handles click-to-select body | — | — |

### Input Systems (PlanetariumUISet)

| System | Purpose | Dependencies |
|--------|---------|--------------|
| `handle_go_to_shortcut` | G/F key triggers GoTo focused body | uses `PhysicsGraph.id_to_entity` |
| `handle_revolve_frame_shortcut` | V key cycles revolve reference frame | — |

### Trajectory Systems (PlanetariumUISet)

```
refresh_precessing_trajectories
              │
              ▼ (before)
     calculate_trajectory
              │
              ▼ (after)
     rebuild_trajectory_caches
              │
              ├──────────────────────┐
              ▼                      ▼
     build_trajectory_meshes    update_focused_trajectory_markers
              │                      │
              │                      ▼
              │              update_mouse_hit_marker
              │                      │
              └──────────────────────┴──► draw_trajectory_marker_labels
```

| System | Purpose | Depends On | Runs Before |
|--------|---------|------------|-------------|
| `spawn_trajectory_meshes_for_bodies` | Creates TrajectoryMesh entities for new bodies | — | — |
| `refresh_precessing_trajectories` | Triggers recalc for precessing orbits | — | before `calculate_trajectory` |
| `calculate_trajectory` | Computes orbital path points (Keplerian) | after `refresh_precessing_trajectories` | — |
| `rebuild_trajectory_caches` | Updates TrajectoryCache from computed points | after `calculate_trajectory` | — |
| `build_trajectory_meshes` | Generates tube mesh geometry | after `position_bodies`, `rebuild_trajectory_caches` | — |
| `update_focused_trajectory_markers` | Updates Pe/Ap marker positions | after `position_bodies` | — |
| `update_mouse_hit_marker` | Updates mouse-hover marker on trajectory | after `position_bodies` | — |
| `draw_trajectory_marker_labels` | Renders marker labels via egui | after `position_bodies`, `update_focused_trajectory_markers`, `update_mouse_hit_marker` | — |
| `cleanup_orphaned_trajectory_meshes` | Despawns meshes for removed bodies | — | — |

### Star Lighting Cache (PlanetariumUISet)

```
position_bodies
      │
      ▼
build_star_lighting_cache ──┬──► update_wireframe_lighting
                            ├──► update_occluder_lighting
                            └──► update_body_points
```

| System | Purpose | Depends On |
|--------|---------|------------|
| `build_star_lighting_cache` | Caches star positions/intensities for frame | after `position_bodies` |

### Body Mesh Systems (PlanetariumUISet)

| System | Purpose | Depends On |
|--------|---------|------------|
| `spawn_body_wireframe_meshes` | Creates wireframe mesh for new bodies | — |
| `spawn_body_occluders` | Creates occluder spheres for new bodies | — |
| `spawn_terminator_meshes` | Creates day/night terminator rings | — |
| `update_terminator_meshes` | Updates terminator positions/orientations | after `orient_bodies` |
| `update_wireframe_lighting` | Updates wireframe sun direction uniforms | after `orient_bodies`, `build_star_lighting_cache` |
| `update_occluder_lighting` | Updates occluder sun direction uniforms | after `orient_bodies`, `build_star_lighting_cache` |
| `update_wireframe_thickness` | Adjusts tube thickness based on camera distance | after `position_bodies` |
| `update_occluder_scale` | Scales occluders to match wireframe | after `update_wireframe_thickness` |
| `cleanup_orphaned_body_wireframes` | Despawns meshes for removed bodies | — |

### Body Point Systems (PlanetariumUISet)

| System | Purpose | Depends On |
|--------|---------|------------|
| `spawn_body_point_meshes` | Creates point-LOD meshes for distant bodies | — |
| `update_body_points` | Updates point visibility/brightness based on distance | after `position_bodies`, `build_star_lighting_cache` |
| `cleanup_orphaned_body_points` | Despawns points for removed bodies | — |
| `label_bodies` | Renders body name labels via egui | after `position_bodies` |

### Celestial Markers (PlanetariumUISet)

| System | Purpose | Depends On |
|--------|---------|------------|
| `spawn_celestial_markers` | Creates Point of Aries, etc. markers | — |
| `update_celestial_markers` | Updates marker positions | after `position_bodies` |

### Starfield (PlanetariumUISet)

| System | Purpose | Run Condition |
|--------|---------|---------------|
| `update_starfield_brightness` | Updates starfield intensity from settings | `resource_changed::<Settings>` |

### Miscellaneous (PlanetariumUISet)

| System | Purpose | Dependencies |
|--------|---------|--------------|
| `adjust_lights` | Adjusts ambient/directional lighting | — |

### EguiPrimaryContextPass (UI Windows)

| System | Purpose |
|--------|---------|
| `mission_clock_widget` | Displays simulation time |
| `control_window` | Play/pause, time controls |
| `body_edit_window` | Edit body properties |
| `body_info_window` | Display body information |
| `settings_window` | In-sim settings panel |
| `spin_window` | Body rotation controls |
| `camera_window` | Camera mode controls |
| `show_hide_panel_widget` | Toggle visibility of UI panels |

### OnExit(AppState::Planetarium)

| System | Purpose |
|--------|---------|
| `unload_simulation_objects` | Despawns all SimulationObject entities |
| `cleanup_celestial_markers` | Despawns celestial reference markers |
| `hide_settings_window_on_planetarium_exit` | Resets EscMenuContext |
| `cleanup_planetarium` | Resets all planetarium state (graph, cache, camera, etc.) |

---

## Escape Menu (EscMenuState)

Runs during AppState::Planetarium.

| Schedule | System | Purpose |
|----------|--------|---------|
| Update | `handle_escape_key` | Opens/closes menu on Escape |
| Update | `track_unsaved_changes` | Monitors for unsaved changes |
| Update | `button_hover_system` | Button hover effects |
| OnEnter(Main) | `setup_main_menu` | Creates main menu UI |
| OnExit(Main) | `cleanup_main_menu` | Removes main menu UI |
| Update (Main) | `handle_main_menu_buttons` | Button click handlers |
| OnEnter(SaveNag) | `setup_save_nag` | Creates "save changes?" dialog |
| OnExit(SaveNag) | `cleanup_save_nag` | Removes dialog |
| Update (SaveNag) | `handle_save_nag_buttons` | Dialog button handlers |
| OnEnter(Naming) | `setup_naming` | Creates save-as naming dialog |
| OnExit(Naming) | `cleanup_naming` | Removes dialog |
| Update (Naming) | `handle_naming_buttons` | Dialog button handlers |
| OnEnter(ConfirmOverwrite) | `setup_confirm_overwrite` | Creates overwrite confirmation |
| OnExit(ConfirmOverwrite) | `cleanup_confirm_overwrite` | Removes dialog |
| Update (ConfirmOverwrite) | `handle_confirm_overwrite_buttons` | Dialog button handlers |
| OnEnter(Closed) | `reset_context_on_close` | Resets menu context |

---

## Debug Systems

| Schedule | System | Run Condition |
|----------|--------|---------------|
| Update | `toggle_perf` | Always (checks F3 key) |
| OnEnter(DebugState::Off) | `despawn_recursive_entities_with::<DebugUI>` | — |
| OnEnter(DebugState::AllPerf) | `add_all_perf` | — |

---

## Data Flow Summary

### Per-Frame Planetarium Pipeline

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            SIMULATION                                        │
├─────────────────────────────────────────────────────────────────────────────┤
│  advance_time ──► calculate_body_positions ──► calculate_trajectory         │
│                            │                           │                     │
│                            ▼                           ▼                     │
│                   rebuild_trajectory_caches ◄──────────┘                     │
└─────────────────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                           PRESENTATION                                       │
├─────────────────────────────────────────────────────────────────────────────┤
│  position_bodies ──► orient_bodies                                          │
│        │                   │                                                 │
│        ├───────────────────┼──► build_star_lighting_cache                   │
│        │                   │              │                                  │
│        │                   │              ├──► update_wireframe_lighting     │
│        │                   │              ├──► update_occluder_lighting      │
│        │                   │              └──► update_body_points            │
│        │                   │                                                 │
│        │                   └──► update_terminator_meshes                     │
│        │                                                                     │
│        ├──► build_trajectory_meshes                                          │
│        ├──► update_focused_trajectory_markers ──┬──► draw_trajectory_marker_labels
│        ├──► update_mouse_hit_marker ────────────┘                            │
│        ├──► update_wireframe_thickness ──► update_occluder_scale            │
│        ├──► update_celestial_markers                                         │
│        └──► label_bodies                                                     │
└─────────────────────────────────────────────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                             CAMERA                                           │
├─────────────────────────────────────────────────────────────────────────────┤
│  run_goto, revolve_around (after calculate_body_positions,                  │
│                            before position_bodies)                           │
│  update_hover_target, pick_body_on_click                                    │
│  player_move, player_look (FreeCam)                                         │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Key Shared Resources

| Resource | Writers | Readers |
|----------|---------|---------|
| `SimTime` | `advance_time`, `load_assets` | Most simulation/presentation systems |
| `PhysicsGraph` | `calculate_body_positions` | `handle_go_to_shortcut`, trajectory systems, camera systems |
| `PositionCache` | `calculate_body_positions` | Trajectory systems |
| `StarLightingFrameCache` | `build_star_lighting_cache` | `update_body_points`, `update_wireframe_lighting`, `update_occluder_lighting` |
| `FocusedBodyState` | UI systems, `pick_body_on_click` | Trajectory systems, label systems |
| `HoverState` | `update_hover_target` | Trajectory marker systems, visibility systems |
| `ViewSettings` | UI systems | Visibility/trajectory systems |
| `Settings` | Settings UI, file load | Display systems, `close_when_requested` |

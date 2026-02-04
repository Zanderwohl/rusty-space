# Archetypes

*List of ECS Archetypes present in the game world.*

---

## Celestial Body Archetypes

All celestial bodies share a common base set of components. Additional components are added based on the `Appearance` variant and `major` flag.

### Base Body Components (All Bodies)

| Component | Source | Description |
|-----------|--------|-------------|
| `SimulationObject` | `src/body/mod.rs` | Marker component for simulation entities |
| `Transform` | Bevy | 3D transform for rendering position |
| `BodyState` | `src/body/motive/info.rs` | Runtime state: current position, velocity, trajectory |
| `Motive` | `src/body/motive/compound_motive.rs` | Time-based motive with transitions (Fixed/Newtonian/Keplerian) |
| `BodyInfo` | `src/body/motive/info.rs` | Static info: name, ID, mass, designation, tags |
| `Appearance` | `src/body/appearance/mod.rs` | Rendering variant: `Empty`, `DebugBall`, or `Star` |
| `Major` or `Minor` | `src/body/universe/mod.rs` | Mutually exclusive markers for gravity calculation |

### Empty Body

Bodies with `Appearance::Empty` have no visual representation.

```
SimulationObject + Transform + BodyState + Motive + BodyInfo + Appearance::Empty + (Major | Minor)
```

**Use case:** Barycenters, reference points, or bodies without visual representation.

### DebugBall Body

Bodies with `Appearance::DebugBall` render as colored spheres.

```
SimulationObject + Transform + BodyState + Motive + BodyInfo + Appearance::DebugBall
+ (Major | Minor) + Mesh3d + MeshMaterial3d<StandardMaterial>
```

**Additional components:**
| Component | Description |
|-----------|-------------|
| `Mesh3d` | Icosphere mesh handle |
| `MeshMaterial3d<StandardMaterial>` | Colored PBR material |

**Use case:** Planets, moons, asteroids, spacecraft.

### Star Body

Bodies with `Appearance::Star` render as luminous spheres with a point light.

```
SimulationObject + Transform + BodyState + Motive + BodyInfo + Appearance::Star
+ (Major | Minor) + Mesh3d + MeshMaterial3d<StandardMaterial> + PointLight + NoFrustumCulling
```

**Additional components:**
| Component | Description |
|-----------|-------------|
| `Mesh3d` | Icosphere mesh handle |
| `MeshMaterial3d<StandardMaterial>` | Emissive PBR material based on stellar properties |
| `PointLight` | Dynamic point light for illumination |
| `NoFrustumCulling` | Ensures stars are always rendered regardless of frustum |

**Use case:** Stars, suns.

---

## Camera Archetype

### Planetarium Camera

The main 3D camera for viewing the simulation.

```
Camera3d + Camera + Hdr + Projection::Perspective + Transform + Freecam + PlanetariumCamera + Bloom + Tonemapping
```

| Component | Source | Description |
|-----------|--------|-------------|
| `Camera3d` | Bevy | 3D camera marker |
| `Camera` | Bevy | Camera settings |
| `Hdr` | Bevy | HDR rendering enabled |
| `Projection::Perspective` | Bevy | Perspective projection with configurable FOV |
| `Transform` | Bevy | Camera orientation (rotation) |
| `Freecam` | `src/gui/util/freecam.rs` | High-precision camera position (`DVec3`) |
| `PlanetariumCamera` | `src/gui/planetarium/camera/mod.rs` | Camera behavior state machine |
| `Bloom` | Bevy | Post-processing bloom effect |
| `Tonemapping` | Bevy | HDR tonemapping (TonyMcMapface) |

**Camera States (`PlanetariumCamera.action`):**
- `Free` – User controls camera directly (WASD + mouse)
- `Goto(GoToInProgress)` – Animated transition to a body
- `RevolveAround(RevolveAround)` – Orbiting around a body

---

## UI Archetypes

### Splash Screen

Displayed briefly on application startup.

```
Node + SplashScreen
  └─ child: ImageNode
```

| Component | Source | Description |
|-----------|--------|-------------|
| `Node` | Bevy UI | UI layout node |
| `SplashScreen` | `src/gui/splash.rs` | Marker for cleanup on state exit |
| `ImageNode` | Bevy UI | Logo image (child entity) |

**Lifetime:** Exists only during `AppState::Splash`.

### Debug UI

Performance metrics overlay (toggled with F3).

```
DebugUI + PerfUiFramerateEntries + PerfUiWindowEntries + PerfUiFixedTimeEntries
```

| Component | Source | Description |
|-----------|--------|-------------|
| `DebugUI` | `src/gui/util/debug.rs` | Marker for cleanup |
| `PerfUiFramerateEntries` | `iyes_perf_ui` | FPS and frame time display |
| `PerfUiWindowEntries` | `iyes_perf_ui` | Window resolution display |
| `PerfUiFixedTimeEntries` | `iyes_perf_ui` | Fixed timestep info display |

**Lifetime:** Exists only during `DebugState::AllPerf`.

---

## Component Details

### Motive System

The `Motive` component stores a time-indexed collection of motive selections, enabling bodies to transition between different physics models:

| Variant | Description |
|---------|-------------|
| `MotiveSelection::Fixed` | Fixed position relative to a parent body or origin |
| `MotiveSelection::Newtonian` | N-body physics, affected by gravity from `Major` bodies |
| `MotiveSelection::Keplerian` | Two-body orbital mechanics around a primary |

**Transition events:**
- `Epoch` – Initial motive at simulation start
- `SOIChange` – Sphere of influence transition
- `Impulse` – Velocity change (maneuver)
- `Release` – Transition from Fixed to Newtonian

### Major vs Minor

Bodies are marked with exactly one of:
- `Major` – Affects gravity calculations (attracts other bodies)
- `Minor` – Does not affect gravity (is only attracted)

This distinction is based on `BodyInfo.major` at spawn time.

### BodyState Fields

| Field | Type | Description |
|-------|------|-------------|
| `current_position` | `DVec3` | Absolute position in simulation space |
| `last_step_position` | `DVec3` | Position at previous physics step |
| `current_velocity` | `Option<DVec3>` | Velocity (Newtonian bodies only) |
| `current_local_position` | `Option<DVec3>` | Position relative to primary |
| `current_primary_position` | `Option<DVec3>` | Primary body's position |
| `trajectory` | `Option<TimeMap<DVec3>>` | Computed future positions |
| `newtonian_init_time` | `Option<f64>` | Time of last Newtonian state initialization |

---

## Query Patterns

Common ECS query patterns used throughout the codebase:

**Body simulation:**
```rust
Query<(&BodyState, &BodyInfo, &Motive)>
Query<(&SimulationObject, &mut Transform, &BodyInfo, &BodyState, &Appearance)>
```

**Star lighting:**
```rust
Query<(&BodyInfo, &mut PointLight, &Appearance)>
```

**Camera control:**
```rust
Query<(&mut Transform, &mut PlanetariumCamera, &mut Freecam)>
Query<(&Camera, &Camera3d, &PlanetariumCamera, &GlobalTransform)>
```

**Gravity (Major bodies only):**
```rust
Query<(&BodyState, &BodyInfo), With<Major>>
```

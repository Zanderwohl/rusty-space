#!/usr/bin/env python3
"""
System DAG Analysis Tool

Builds a directed acyclic graph (DAG) of all Bevy systems in the rusty-space
project and provides topology analysis utilities.

Usage:
    python system_dag.py [command]

Commands:
    topo        Print topological sort of all systems
    longest     Find the longest path (critical path) in the DAG
    paths       Find all paths between two systems
    deps        Show all dependencies for a system
    dependents  Show all systems that depend on a given system
    dot         Output GraphViz DOT format for visualization
    stats       Print statistics about the graph
    validate    Check for cycles and other issues

Requirements:
    pip install networkx matplotlib
"""

import argparse
from collections import defaultdict
from dataclasses import dataclass, field
from enum import Enum, auto
from typing import Optional

try:
    import networkx as nx
    HAS_NETWORKX = True
except ImportError:
    HAS_NETWORKX = False
    print("Warning: networkx not installed. Install with: pip install networkx")

try:
    import matplotlib.pyplot as plt
    HAS_MATPLOTLIB = True
except ImportError:
    HAS_MATPLOTLIB = False


class Schedule(Enum):
    STARTUP = auto()
    UPDATE = auto()
    ON_ENTER = auto()
    ON_EXIT = auto()
    EGUI_PRIMARY_CONTEXT_PASS = auto()


class AppState(Enum):
    GLOBAL = auto()  # Always runs
    SPLASH = auto()
    MAIN_MENU = auto()
    PLANETARIUM_LOADING = auto()
    PLANETARIUM = auto()


@dataclass
class System:
    """Represents a Bevy system with its metadata."""
    name: str
    schedule: Schedule = Schedule.UPDATE
    app_state: AppState = AppState.GLOBAL
    system_set: Optional[str] = None
    description: str = ""
    after: list[str] = field(default_factory=list)
    before: list[str] = field(default_factory=list)
    run_condition: Optional[str] = None


# =============================================================================
# SYSTEM DEFINITIONS
# =============================================================================

SYSTEMS: list[System] = [
    # =========================================================================
    # STARTUP SYSTEMS
    # =========================================================================
    System(
        name="common_setup",
        schedule=Schedule.STARTUP,
        description="Spawns camera with Freecam, PlanetariumCamera, Bloom",
    ),
    System(
        name="load_catalogs",
        schedule=Schedule.STARTUP,
        description="Loads star catalog data from files",
    ),
    System(
        name="spawn_starfield",
        schedule=Schedule.STARTUP,
        description="Creates starfield mesh and material",
        after=["load_catalogs"],
    ),
    System(
        name="initial_grab_on_flycam_spawn",
        schedule=Schedule.STARTUP,
        description="Sets up initial cursor grab state",
    ),

    # =========================================================================
    # GLOBAL UPDATE SYSTEMS (always run)
    # =========================================================================
    System(
        name="close_when_requested",
        description="Handles window close, saves settings on change/close",
    ),
    System(
        name="make_visible",
        description="Makes window visible after frame 3",
    ),
    System(
        name="update_post_process_settings",
        description="Updates bloom/tonemapping",
        run_condition="resource_changed::<PostProcessSettings>",
    ),
    System(
        name="toggle_perf",
        description="Toggles debug performance UI",
    ),

    # FreeCam systems (always active during Update)
    System(
        name="player_move",
        description="WASD movement for free camera",
    ),
    System(
        name="player_look",
        description="Mouse look for free camera",
    ),
    System(
        name="cursor_grab",
        description="Handles cursor grab toggle",
    ),

    # =========================================================================
    # SPLASH SCREEN
    # =========================================================================
    System(
        name="splash_setup",
        schedule=Schedule.ON_ENTER,
        app_state=AppState.SPLASH,
        description="Creates splash screen UI",
    ),
    System(
        name="countdown",
        app_state=AppState.SPLASH,
        description="Counts down and transitions to MainMenu",
    ),

    # =========================================================================
    # MAIN MENU
    # =========================================================================
    System(
        name="main_menu",
        schedule=Schedule.EGUI_PRIMARY_CONTEXT_PASS,
        app_state=AppState.MAIN_MENU,
        description="Renders home menu buttons",
        run_condition="MenuState::Home",
    ),
    System(
        name="planetarium_menu",
        schedule=Schedule.EGUI_PRIMARY_CONTEXT_PASS,
        app_state=AppState.MAIN_MENU,
        description="Renders save/load file picker",
        run_condition="MenuState::Planetarium",
    ),
    System(
        name="settings_menu",
        schedule=Schedule.EGUI_PRIMARY_CONTEXT_PASS,
        app_state=AppState.MAIN_MENU,
        description="Renders settings panel",
        run_condition="MenuState::Settings",
    ),
    System(
        name="load_planetarium_files",
        schedule=Schedule.ON_ENTER,
        app_state=AppState.MAIN_MENU,
        description="Scans data/templates and data/saves",
    ),
    System(
        name="quit_system",
        app_state=AppState.MAIN_MENU,
        description="Handles quit request from UI",
    ),

    # =========================================================================
    # PLANETARIUM LOADING
    # =========================================================================
    System(
        name="load_assets",
        app_state=AppState.PLANETARIUM_LOADING,
        system_set="PlanetariumLoadingSet",
        description="Loads universe file, spawns bodies",
    ),
    System(
        name="initial_trajectories",
        schedule=Schedule.ON_EXIT,
        app_state=AppState.PLANETARIUM_LOADING,
        description="Requests trajectory calculation for all bodies",
    ),

    # =========================================================================
    # PLANETARIUM - CORE POSITION PIPELINE
    # =========================================================================
    System(
        name="calculate_body_positions",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Advances time and computes simulation positions via PhysicsGraph",
    ),
    System(
        name="position_bodies",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Copies simulation positions to Bevy Transforms",
        after=["calculate_body_positions"],
    ),
    System(
        name="orient_bodies",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Applies body rotation/tilt",
        after=["position_bodies"],
    ),

    # =========================================================================
    # PLANETARIUM - CAMERA
    # =========================================================================
    System(
        name="handle_gotos",
        app_state=AppState.PLANETARIUM,
        description="Processes GoTo messages, starts camera transitions",
    ),
    System(
        name="run_goto",
        app_state=AppState.PLANETARIUM,
        description="Animates camera GoTo movement",
        after=["calculate_body_positions"],
    ),
    System(
        name="revolve_around",
        app_state=AppState.PLANETARIUM,
        description="Updates camera revolve-around-body behavior",
        after=["calculate_body_positions"],
    ),
    System(
        name="update_hover_target",
        app_state=AppState.PLANETARIUM,
        description="Raycasts for body/trajectory hover detection",
    ),
    System(
        name="pick_body_on_click",
        app_state=AppState.PLANETARIUM,
        description="Handles click-to-select body",
    ),

    # =========================================================================
    # PLANETARIUM - INPUT
    # =========================================================================
    System(
        name="handle_go_to_shortcut",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="G/F key triggers GoTo focused body",
    ),
    System(
        name="handle_revolve_frame_shortcut",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="V key cycles revolve reference frame",
    ),

    # =========================================================================
    # PLANETARIUM - TRAJECTORY SYSTEMS
    # =========================================================================
    System(
        name="spawn_trajectory_meshes_for_bodies",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Creates TrajectoryMesh entities for new bodies",
    ),
    System(
        name="refresh_precessing_trajectories",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Triggers recalc for precessing orbits",
        before=["calculate_trajectory"],
    ),
    System(
        name="calculate_trajectory",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Computes orbital path points (Keplerian)",
    ),
    System(
        name="rebuild_trajectory_caches",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Updates TrajectoryCache from computed points",
        after=["calculate_trajectory"],
    ),
    System(
        name="build_trajectory_meshes",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Generates tube mesh geometry",
        after=["position_bodies", "rebuild_trajectory_caches"],
    ),
    System(
        name="update_focused_trajectory_markers",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Updates Pe/Ap marker positions",
        after=["position_bodies"],
    ),
    System(
        name="update_mouse_hit_marker",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Updates mouse-hover marker on trajectory",
        after=["position_bodies"],
    ),
    System(
        name="draw_trajectory_marker_labels",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Renders marker labels via egui",
        after=["position_bodies", "update_focused_trajectory_markers", "update_mouse_hit_marker"],
    ),
    System(
        name="cleanup_orphaned_trajectory_meshes",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Despawns meshes for removed bodies",
    ),

    # =========================================================================
    # PLANETARIUM - STAR LIGHTING CACHE
    # =========================================================================
    System(
        name="build_star_lighting_cache",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Caches star positions/intensities for frame",
        after=["position_bodies"],
    ),

    # =========================================================================
    # PLANETARIUM - BODY MESH SYSTEMS
    # =========================================================================
    System(
        name="spawn_body_wireframe_meshes",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Creates wireframe mesh for new bodies",
    ),
    System(
        name="spawn_body_occluders",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Creates occluder spheres for new bodies",
    ),
    System(
        name="spawn_terminator_meshes",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Creates day/night terminator rings",
    ),
    System(
        name="update_terminator_meshes",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Updates terminator positions/orientations",
        after=["orient_bodies"],
    ),
    System(
        name="update_wireframe_lighting",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Updates wireframe sun direction uniforms",
        after=["orient_bodies", "build_star_lighting_cache"],
    ),
    System(
        name="update_occluder_lighting",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Updates occluder sun direction uniforms",
        after=["orient_bodies", "build_star_lighting_cache"],
    ),
    System(
        name="update_wireframe_thickness",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Adjusts tube thickness based on camera distance",
        after=["position_bodies"],
    ),
    System(
        name="update_occluder_scale",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Scales occluders to match wireframe",
        after=["update_wireframe_thickness"],
    ),
    System(
        name="cleanup_orphaned_body_wireframes",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Despawns meshes for removed bodies",
    ),

    # =========================================================================
    # PLANETARIUM - BODY POINT SYSTEMS
    # =========================================================================
    System(
        name="spawn_body_point_meshes",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Creates point-LOD meshes for distant bodies",
    ),
    System(
        name="update_body_points",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Updates point visibility/brightness based on distance",
        after=["position_bodies", "build_star_lighting_cache"],
    ),
    System(
        name="cleanup_orphaned_body_points",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Despawns points for removed bodies",
    ),
    System(
        name="label_bodies",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Renders body name labels via egui",
        after=["position_bodies"],
    ),

    # =========================================================================
    # PLANETARIUM - CELESTIAL MARKERS
    # =========================================================================
    System(
        name="spawn_celestial_markers",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Creates Point of Aries, etc. markers",
    ),
    System(
        name="update_celestial_markers",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Updates marker positions",
        after=["position_bodies"],
    ),

    # =========================================================================
    # PLANETARIUM - STARFIELD
    # =========================================================================
    System(
        name="update_starfield_brightness",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Updates starfield intensity from settings",
        run_condition="resource_changed::<Settings>",
    ),

    # =========================================================================
    # PLANETARIUM - MISC
    # =========================================================================
    System(
        name="adjust_lights",
        app_state=AppState.PLANETARIUM,
        system_set="PlanetariumUISet",
        description="Adjusts ambient/directional lighting",
    ),

    # =========================================================================
    # PLANETARIUM - UI WINDOWS (EguiPrimaryContextPass)
    # =========================================================================
    System(
        name="mission_clock_widget",
        schedule=Schedule.EGUI_PRIMARY_CONTEXT_PASS,
        app_state=AppState.PLANETARIUM,
        description="Displays simulation time",
    ),
    System(
        name="control_window",
        schedule=Schedule.EGUI_PRIMARY_CONTEXT_PASS,
        app_state=AppState.PLANETARIUM,
        description="Play/pause, time controls",
    ),
    System(
        name="body_edit_window",
        schedule=Schedule.EGUI_PRIMARY_CONTEXT_PASS,
        app_state=AppState.PLANETARIUM,
        description="Edit body properties",
    ),
    System(
        name="body_info_window",
        schedule=Schedule.EGUI_PRIMARY_CONTEXT_PASS,
        app_state=AppState.PLANETARIUM,
        description="Display body information",
    ),
    System(
        name="settings_window",
        schedule=Schedule.EGUI_PRIMARY_CONTEXT_PASS,
        app_state=AppState.PLANETARIUM,
        description="In-sim settings panel",
    ),
    System(
        name="spin_window",
        schedule=Schedule.EGUI_PRIMARY_CONTEXT_PASS,
        app_state=AppState.PLANETARIUM,
        description="Body rotation controls",
    ),
    System(
        name="camera_window",
        schedule=Schedule.EGUI_PRIMARY_CONTEXT_PASS,
        app_state=AppState.PLANETARIUM,
        description="Camera mode controls",
    ),
    System(
        name="show_hide_panel_widget",
        schedule=Schedule.EGUI_PRIMARY_CONTEXT_PASS,
        app_state=AppState.PLANETARIUM,
        description="Toggle visibility of UI panels",
    ),

    # =========================================================================
    # PLANETARIUM - CLEANUP (OnExit)
    # =========================================================================
    System(
        name="unload_simulation_objects",
        schedule=Schedule.ON_EXIT,
        app_state=AppState.PLANETARIUM,
        description="Despawns all SimulationObject entities",
    ),
    System(
        name="cleanup_celestial_markers",
        schedule=Schedule.ON_EXIT,
        app_state=AppState.PLANETARIUM,
        description="Despawns celestial reference markers",
    ),
    System(
        name="hide_settings_window_on_planetarium_exit",
        schedule=Schedule.ON_EXIT,
        app_state=AppState.PLANETARIUM,
        description="Resets EscMenuContext",
    ),
    System(
        name="cleanup_planetarium",
        schedule=Schedule.ON_EXIT,
        app_state=AppState.PLANETARIUM,
        description="Resets all planetarium state",
    ),

    # =========================================================================
    # ESCAPE MENU
    # =========================================================================
    System(
        name="handle_escape_key",
        app_state=AppState.PLANETARIUM,
        description="Opens/closes menu on Escape",
    ),
    System(
        name="track_unsaved_changes",
        app_state=AppState.PLANETARIUM,
        description="Monitors for unsaved changes",
    ),
    System(
        name="button_hover_system",
        app_state=AppState.PLANETARIUM,
        description="Button hover effects",
    ),
]


# =============================================================================
# GRAPH BUILDING
# =============================================================================

def build_graph(
    systems: list[System],
    app_state: Optional[AppState] = None,
    schedule: Optional[Schedule] = None,
) -> "nx.DiGraph":
    """Build a NetworkX DiGraph from system definitions.
    
    Args:
        systems: List of System objects
        app_state: Filter to only include systems in this app state (None = all)
        schedule: Filter to only include systems in this schedule (None = all)
    
    Returns:
        NetworkX DiGraph with systems as nodes and dependencies as edges
    """
    if not HAS_NETWORKX:
        raise ImportError("networkx is required. Install with: pip install networkx")
    
    G = nx.DiGraph()
    
    # Build name -> system lookup
    system_map = {s.name: s for s in systems}
    
    # Filter systems
    filtered = systems
    if app_state is not None:
        filtered = [s for s in filtered if s.app_state == app_state or s.app_state == AppState.GLOBAL]
    if schedule is not None:
        filtered = [s for s in filtered if s.schedule == schedule]
    
    # Add nodes
    for sys in filtered:
        G.add_node(
            sys.name,
            schedule=sys.schedule.name,
            app_state=sys.app_state.name,
            system_set=sys.system_set or "",
            description=sys.description,
            run_condition=sys.run_condition or "",
        )
    
    # Add edges (dependencies)
    for sys in filtered:
        # "after" means this system runs after the dependency
        for dep in sys.after:
            if dep in G.nodes:
                G.add_edge(dep, sys.name, constraint="after")
        
        # "before" means this system runs before the target
        for target in sys.before:
            if target in G.nodes:
                G.add_edge(sys.name, target, constraint="before")
    
    return G


def build_planetarium_update_graph() -> "nx.DiGraph":
    """Build graph of just Planetarium Update systems (the main simulation loop)."""
    return build_graph(
        SYSTEMS,
        app_state=AppState.PLANETARIUM,
        schedule=Schedule.UPDATE,
    )


# =============================================================================
# ANALYSIS FUNCTIONS
# =============================================================================

def topological_sort(G: "nx.DiGraph") -> list[str]:
    """Return systems in topological order (respecting dependencies)."""
    return list(nx.topological_sort(G))


def longest_path(G: "nx.DiGraph") -> tuple[list[str], int]:
    """Find the longest path (critical path) in the DAG.
    
    Returns:
        Tuple of (path as list of node names, path length)
    """
    path = nx.dag_longest_path(G)
    return path, len(path) - 1


def all_paths(G: "nx.DiGraph", source: str, target: str) -> list[list[str]]:
    """Find all paths between two systems."""
    return list(nx.all_simple_paths(G, source, target))


def get_dependencies(G: "nx.DiGraph", system: str, transitive: bool = False) -> set[str]:
    """Get all systems that must run before the given system.
    
    Args:
        G: The graph
        system: System name to query
        transitive: If True, include transitive dependencies (dependencies of dependencies)
    """
    if transitive:
        return nx.ancestors(G, system)
    else:
        return set(G.predecessors(system))


def get_dependents(G: "nx.DiGraph", system: str, transitive: bool = False) -> set[str]:
    """Get all systems that depend on the given system.
    
    Args:
        G: The graph
        system: System name to query
        transitive: If True, include transitive dependents
    """
    if transitive:
        return nx.descendants(G, system)
    else:
        return set(G.successors(system))


def find_roots(G: "nx.DiGraph") -> list[str]:
    """Find systems with no dependencies (can run first)."""
    return [n for n in G.nodes if G.in_degree(n) == 0]


def find_leaves(G: "nx.DiGraph") -> list[str]:
    """Find systems with no dependents (run last)."""
    return [n for n in G.nodes if G.out_degree(n) == 0]


def parallel_levels(G: "nx.DiGraph") -> list[list[str]]:
    """Group systems into parallel execution levels.
    
    Systems in the same level have no dependencies on each other
    and can theoretically run in parallel.
    """
    # Use topological generations
    return list(nx.topological_generations(G))


def graph_stats(G: "nx.DiGraph") -> dict:
    """Compute various statistics about the graph."""
    path, length = longest_path(G)
    levels = parallel_levels(G)
    
    return {
        "total_systems": G.number_of_nodes(),
        "total_edges": G.number_of_edges(),
        "roots": find_roots(G),
        "leaves": find_leaves(G),
        "longest_path": path,
        "longest_path_length": length,
        "parallel_levels": len(levels),
        "max_parallel_width": max(len(level) for level in levels) if levels else 0,
        "is_dag": nx.is_directed_acyclic_graph(G),
    }


# =============================================================================
# OUTPUT FUNCTIONS
# =============================================================================

def to_dot(G: "nx.DiGraph", title: str = "System DAG") -> str:
    """Generate GraphViz DOT format string."""
    lines = [
        f'digraph "{title}" {{',
        '    rankdir=TB;',
        '    node [shape=box, style=filled, fillcolor=lightblue];',
        '',
    ]
    
    # Group by system set
    sets = defaultdict(list)
    for node in G.nodes:
        system_set = G.nodes[node].get("system_set", "")
        sets[system_set].append(node)
    
    # Add subgraphs for each system set
    for set_name, nodes in sets.items():
        if set_name:
            lines.append(f'    subgraph cluster_{set_name} {{')
            lines.append(f'        label="{set_name}";')
            lines.append('        style=dashed;')
            for node in nodes:
                desc = G.nodes[node].get("description", "")
                lines.append(f'        "{node}" [tooltip="{desc}"];')
            lines.append('    }')
        else:
            for node in nodes:
                desc = G.nodes[node].get("description", "")
                lines.append(f'    "{node}" [tooltip="{desc}"];')
    
    lines.append('')
    
    # Add edges
    for u, v in G.edges:
        lines.append(f'    "{u}" -> "{v}";')
    
    lines.append('}')
    return '\n'.join(lines)


def print_parallel_levels(G: "nx.DiGraph"):
    """Print systems grouped by parallel execution level."""
    levels = parallel_levels(G)
    for i, level in enumerate(levels):
        print(f"Level {i}: {', '.join(sorted(level))}")


# =============================================================================
# CLI
# =============================================================================

def main():
    parser = argparse.ArgumentParser(
        description="Analyze the Bevy system dependency graph",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    
    subparsers = parser.add_subparsers(dest="command", help="Command to run")
    
    # topo command
    subparsers.add_parser("topo", help="Print topological sort of all systems")
    
    # longest command
    subparsers.add_parser("longest", help="Find the longest path (critical path)")
    
    # paths command
    paths_parser = subparsers.add_parser("paths", help="Find all paths between two systems")
    paths_parser.add_argument("source", help="Source system name")
    paths_parser.add_argument("target", help="Target system name")
    
    # deps command
    deps_parser = subparsers.add_parser("deps", help="Show dependencies for a system")
    deps_parser.add_argument("system", help="System name")
    deps_parser.add_argument("-t", "--transitive", action="store_true", help="Include transitive deps")
    
    # dependents command
    dependents_parser = subparsers.add_parser("dependents", help="Show systems that depend on a given system")
    dependents_parser.add_argument("system", help="System name")
    dependents_parser.add_argument("-t", "--transitive", action="store_true", help="Include transitive dependents")
    
    # dot command
    dot_parser = subparsers.add_parser("dot", help="Output GraphViz DOT format")
    dot_parser.add_argument("-o", "--output", help="Output file (default: stdout)")
    
    # stats command
    subparsers.add_parser("stats", help="Print statistics about the graph")
    
    # validate command
    subparsers.add_parser("validate", help="Check for cycles and other issues")
    
    # levels command
    subparsers.add_parser("levels", help="Show parallel execution levels")
    
    # list command
    subparsers.add_parser("list", help="List all systems")
    
    args = parser.parse_args()
    
    if not HAS_NETWORKX:
        print("Error: networkx is required. Install with: pip install networkx")
        return 1
    
    # Build the main planetarium update graph by default
    G = build_planetarium_update_graph()
    
    if args.command == "topo":
        print("Topological order of Planetarium Update systems:")
        print("-" * 50)
        for i, sys in enumerate(topological_sort(G), 1):
            print(f"{i:3}. {sys}")
    
    elif args.command == "longest":
        path, length = longest_path(G)
        print(f"Longest path (critical path) - {length} edges:")
        print("-" * 50)
        for i, sys in enumerate(path):
            prefix = "  " if i == 0 else "  └─► "
            print(f"{prefix}{sys}")
    
    elif args.command == "paths":
        if args.source not in G.nodes:
            print(f"Error: System '{args.source}' not found")
            return 1
        if args.target not in G.nodes:
            print(f"Error: System '{args.target}' not found")
            return 1
        
        paths = all_paths(G, args.source, args.target)
        print(f"All paths from '{args.source}' to '{args.target}':")
        print("-" * 50)
        if not paths:
            print("No paths found")
        else:
            for i, path in enumerate(paths, 1):
                print(f"{i}. {' → '.join(path)}")
    
    elif args.command == "deps":
        if args.system not in G.nodes:
            print(f"Error: System '{args.system}' not found")
            return 1
        
        deps = get_dependencies(G, args.system, transitive=args.transitive)
        kind = "Transitive dependencies" if args.transitive else "Direct dependencies"
        print(f"{kind} of '{args.system}':")
        print("-" * 50)
        if not deps:
            print("None")
        else:
            for dep in sorted(deps):
                print(f"  - {dep}")
    
    elif args.command == "dependents":
        if args.system not in G.nodes:
            print(f"Error: System '{args.system}' not found")
            return 1
        
        deps = get_dependents(G, args.system, transitive=args.transitive)
        kind = "Transitive dependents" if args.transitive else "Direct dependents"
        print(f"{kind} of '{args.system}':")
        print("-" * 50)
        if not deps:
            print("None")
        else:
            for dep in sorted(deps):
                print(f"  - {dep}")
    
    elif args.command == "dot":
        dot = to_dot(G, "Planetarium Update Systems")
        if args.output:
            with open(args.output, 'w') as f:
                f.write(dot)
            print(f"Written to {args.output}")
        else:
            print(dot)
    
    elif args.command == "stats":
        stats = graph_stats(G)
        print("Graph Statistics (Planetarium Update)")
        print("=" * 50)
        print(f"Total systems: {stats['total_systems']}")
        print(f"Total dependency edges: {stats['total_edges']}")
        print(f"Is valid DAG: {stats['is_dag']}")
        print(f"Parallel levels: {stats['parallel_levels']}")
        print(f"Max parallel width: {stats['max_parallel_width']}")
        print()
        print(f"Roots (no dependencies): {len(stats['roots'])}")
        for r in sorted(stats['roots']):
            print(f"  - {r}")
        print()
        print(f"Leaves (no dependents): {len(stats['leaves'])}")
        for l in sorted(stats['leaves']):
            print(f"  - {l}")
        print()
        print(f"Critical path ({stats['longest_path_length']} edges):")
        for i, sys in enumerate(stats['longest_path']):
            prefix = "  " if i == 0 else "  └─► "
            print(f"{prefix}{sys}")
    
    elif args.command == "validate":
        print("Validating graph...")
        print("-" * 50)
        
        # Check for cycles
        if nx.is_directed_acyclic_graph(G):
            print("✓ No cycles detected")
        else:
            print("✗ CYCLES DETECTED!")
            cycles = list(nx.simple_cycles(G))
            for cycle in cycles:
                print(f"  Cycle: {' → '.join(cycle)}")
        
        # Check for missing dependencies
        all_names = {s.name for s in SYSTEMS}
        for sys in SYSTEMS:
            for dep in sys.after + sys.before:
                if dep not in all_names:
                    print(f"✗ System '{sys.name}' references unknown system '{dep}'")
        
        print("✓ Validation complete")
    
    elif args.command == "levels":
        print("Parallel execution levels:")
        print("-" * 50)
        print_parallel_levels(G)
    
    elif args.command == "list":
        print("All systems:")
        print("-" * 50)
        for sys in sorted(SYSTEMS, key=lambda s: (s.app_state.name, s.schedule.name, s.name)):
            print(f"[{sys.app_state.name:20}] [{sys.schedule.name:10}] {sys.name}")
    
    else:
        parser.print_help()
    
    return 0


if __name__ == "__main__":
    exit(main())

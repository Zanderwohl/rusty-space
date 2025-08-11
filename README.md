# Exotic Matters Engine

Squishy People and Hard Science

A program for course plotting and mission simulation for the Tabletop RPG Exotic Matters:
A scientific, mathematic TTRPG by Alexander Lowry

![Image credit: Orion Vehicle Design from Nuclear Pulse Space Vehicle Operational
Study, Volume III – Conceptual Vehicle Designs and Operational Systems, 1964](readme/title-card.png)

This TTRPG comes with a program to plot and calculate trajectories,
plus [a book](books/ExoticMatters-Rulebook.pdf) containing the rules for play.

**Both are incomplete right now!**
Neither are completely workable yet.

![The inner solar system, orbiting with accelerated time along green trajectory lines.](readme/solar_system_01.gif)

## Building the program

Assuming you have cargo and Rust (>=1.89.0) installed: `cargo run --bin exotic_matters --release`.
First-time compilation will take several minutes: about 15 minutes on my M3 Mac, and 30 minutes on my Ryzen 7.

![Camera sweeps from a green ball Earth out to Dysnomia, a small moon of Eris at the edge of the solar system.](readme/to_eris.gif)

## To Do List

* Add textures to objects
* Rotation and quaternion stuff (sadge)
* Procedural textures for planets
* Procedural textures for ring worlds
* Dyson spheres
* ring wolds
* potato asteroids
* rosettes
* klemperer rosettes
* let things change course over time (impulses)
* SOI changes
* Make ring habs scale properly with distance/object scale for symbolic views
* Bouncy animations for changing size
* Automatic size presets for useful stuff
* Information about trajectory lines on hover
* Little spacecraft type of object

## Bug List



## Cool Fonts

* Futura
* Berkeley Mono

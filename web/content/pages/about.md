+++
title = "About"
summary = "What Lightcone is, who is building it, and what state it is in."
+++

Lightcone is a one-person project, built in the open, on top of an orbital mechanics library
written for something else entirely.

That something else is *Exotic Matters*, a trajectory tool for a tabletop game. Its Keplerian
propagation, its reference frames and its epoch handling are checked against JPL Horizons, and
they turned out to be most of what a space game needs. The shared libraries are engine-free
and belong to neither product; the game-specific code sits above them.

## State of play

The spacetime layer, the photometry, the sky and a single-process client all exist and run.
The premise has been tested in the smallest program that could disprove it: an observer thirty
light-years out sees a swarm change thirty years late, and not before.

What does not exist yet: the server, the browser build, and the game on top of them. This site
is the first of those three.

## Reading

Every design decision is written down before it is built, including the ones that were wrong
and had to be revised. The documents are public, and the buildout plan is the order of work.

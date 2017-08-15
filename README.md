# Beyond Perception - audio_server

An Audio Server for Scienceworks' Beyond Perception Exhibition


## Overview

The audio server runs on a single machine and is responsible for the following:

- Stores all audio content.
- Plays back audio for all installations over the network via [Dante](https://www.audinate.com/).
- Analyses audio channels and sends data to installation computers via OSC.
- Responds to interactive feedback/triggers from relevant installations via OSC.
- Runs the generative audio system.
- Runs the control GUI.

### INPUTS

- Installation Interactions:
    - Energetic Vibrations - ID and Pressure per Transducer Chair
    - Ripples In Spacetime - Depth Camera ID and Data
    - Turbulent Encounters - Fog Sculpture & Fluid Dynamics

### OUTPUTS

- Audio - Dante Virtual Audio Soundcard
- Audio Analysis - Over network to target installation computers via OSC


## Code - Rust

All code is written in [The Rust Programming
Language](https://www.rust-lang.org/) for real-time performance, memory safety,
a modern type system and a standard package manager. Find more information about
Rust here:

- [Official site](https://www.rust-lang.org/)
- [The Rust Programming Language](https://doc.rust-lang.org/book/) online book
- [The STD Library Reference](https://doc.rust-lang.org/std/)
- #rust at irc.mozilla.org - lots of friendly folk willing to help
- [r4cppp](https://github.com/nrc/r4cppp) - a tutorial for experienced C and C++
  programmers

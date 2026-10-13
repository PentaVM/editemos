//! Open-source motion graphics: deterministic HTML scenes rendered frame by frame
//! through the machine's own Chromium, encoded with ffmpeg to video WITH ALPHA.
//! No After Effects, no cloud, no render farm.

pub mod fonts;
pub mod library;
pub mod lint;
pub mod render;
pub mod templates;

pub const LIB_JS: &str = include_str!("../../assets/motion-lib.js");

//! Core of uBot Manager: the package index, the project model, problems, planning and task
//! execution. The Tauri shell and the `ubot` CLI use it through [`manager::Manager`].
//!
//! `docs/SPEC.md` in the manager repository is the contract this crate implements.

pub mod descriptor;
pub mod download;
pub mod editor;
pub mod engine;
pub mod env;
pub mod exec;
pub mod git;
pub mod graph;
pub mod index;
pub mod manager;
pub mod model;
pub mod msg;
pub mod paths;
pub mod plan;
pub mod prebuilt;
pub mod problems;
pub mod process;
pub mod project;
pub mod recipes;
pub mod settings;
pub mod sources;
pub mod status;
pub mod update;
pub mod uproject;
pub mod url;
pub mod version;
pub mod workspace;

pub use exec::{Remover, TaskSink};
pub use manager::{ApiResult, Manager, ManagerOptions};

// SPDX-License-Identifier: GPL-3.0-or-later
//! battbar core: data model, provider trait and polling engine, source merging,
//! device database and the frame transport abstraction.

pub mod calibration;
pub mod cells;
pub mod charge;
pub mod devicedb;
pub mod ids;
pub mod learner;
pub mod merge;
pub mod model;
pub mod objc_pool;
pub mod provider;
pub mod settings;
pub mod transport;
pub mod version;

pub use charge::{Estimate, SettlingTracker};
pub use devicedb::DeviceDb;
pub use ids::keyed_hash;
pub use learner::LinkLearner;
pub use merge::{merge, Device, IdentityMemory, MergeConfig, NewDeviceGate};
pub use model::*;
pub use provider::{dev_no_hotplug, BatteryProvider, Engine, EventSink, PollResult, Rescan, SAFETY_RESCAN};
pub use settings::{DisplayMode, LearnedLink, Settings};
pub use transport::{FrameTransport, LinkInfo, TransportError};

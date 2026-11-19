// SPDX-License-Identifier: GPL-3.0-or-later
//! battbar core: data model, provider trait and polling engine, source merging,
//! device database and the frame transport abstraction.

pub mod devicedb;
pub mod learner;
pub mod merge;
pub mod model;
pub mod provider;
pub mod settings;
pub mod transport;

pub use devicedb::DeviceDb;
pub use learner::LinkLearner;
pub use merge::{merge, Device, IdentityMemory, MergeConfig};
pub use model::*;
pub use provider::{BatteryProvider, Engine, EventSink, PollResult};
pub use settings::{LearnedLink, Settings};
pub use transport::{FrameTransport, LinkInfo, TransportError};

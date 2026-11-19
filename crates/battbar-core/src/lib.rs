// SPDX-License-Identifier: GPL-3.0-or-later
//! battbar core: data model, provider trait and polling engine, source merging,
//! device database and the frame transport abstraction.

pub mod devicedb;
pub mod merge;
pub mod model;
pub mod provider;
pub mod transport;

pub use devicedb::DeviceDb;
pub use merge::{merge, Device, MergeConfig};
pub use model::*;
pub use provider::{BatteryProvider, Engine, EventSink, PollResult};
pub use transport::{FrameTransport, LinkInfo, TransportError};

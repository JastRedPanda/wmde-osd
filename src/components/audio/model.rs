// Copyright 2026 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

use std::collections::HashMap;

use cosmic_settings_audio_client::{self as audio_client};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NodeVolume {
    pub volume: u32,
    pub mute: bool,
}

#[derive(Debug, Default)]
pub struct Model {
    sinks: Nodes,
    sources: Nodes,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct NodeId(u32);

#[derive(Debug, Default)]
struct Node {
    volume: Option<u32>,
    mute: Option<bool>,
}

impl Node {
    fn value(&self) -> Option<NodeVolume> {
        Some(NodeVolume {
            volume: self.volume?,
            mute: self.mute?,
        })
    }
}

#[derive(Debug, Default)]
struct Nodes {
    active: Option<NodeId>,
    nodes: HashMap<NodeId, Node>,
    /// WMDE: the last value of the default node that was reported. A change is a
    /// difference from it, not a node turning up: a device that vanishes and comes
    /// back with a new id, as on every VT switch away and back (logind revokes the
    /// session's audio devices meanwhile), reads as a fresh node and used to pop the
    /// indicator up.
    reported: Option<NodeVolume>,
}

impl Nodes {
    fn active(&self) -> Option<NodeVolume> {
        self.nodes.get(&self.active?)?.value()
    }

    fn remove(&mut self, node_id: NodeId) -> bool {
        self.active.take_if(|active| *active == node_id);
        self.nodes.remove(&node_id).is_some()
    }

    fn node_mut(&mut self, node_id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(&node_id)
    }

    /// The default node's value, if it differs from the one reported last.
    fn changed(&mut self) -> Option<NodeVolume> {
        let value = self.active()?;
        if self.reported == Some(value) {
            return None;
        }
        self.reported = Some(value);
        Some(value)
    }
}

pub enum Response {
    SinkVolume(NodeVolume),
    SourceVolume(NodeVolume),
}

impl Model {
    pub fn update(&mut self, event: audio_client::Event) -> Option<Response> {
        match event {
            audio_client::Event::NodeMute(node_id, mute) => {
                let node_id = NodeId(node_id);
                if let Some(node) = self.sinks.node_mut(node_id) {
                    node.mute = Some(mute);
                } else if let Some(node) = self.sources.node_mut(node_id) {
                    node.mute = Some(mute);
                }
            }

            audio_client::Event::NodeVolume(node_id, volume, _balance) => {
                let node_id = NodeId(node_id);
                if let Some(node) = self.sinks.node_mut(node_id) {
                    node.volume = Some(volume);
                } else if let Some(node) = self.sources.node_mut(node_id) {
                    node.volume = Some(volume);
                }
            }

            audio_client::Event::DefaultSink(node_id) => {
                self.sinks.active = Some(NodeId(node_id));
            }

            audio_client::Event::DefaultSource(node_id) => {
                self.sources.active = Some(NodeId(node_id));
            }

            audio_client::Event::Node(node_id, node) => {
                let nodes = if node.is_sink {
                    &mut self.sinks
                } else {
                    &mut self.sources
                };
                nodes.nodes.entry(NodeId(node_id)).or_default();
            }

            audio_client::Event::RemoveNode(node_id) => {
                let node_id = NodeId(node_id);
                if !self.sinks.remove(node_id) {
                    self.sources.remove(node_id);
                }
            }

            _ => return None,
        }

        // One event touches one direction, so at most one of these fires.
        if let Some(value) = self.sinks.changed() {
            return Some(Response::SinkVolume(value));
        }
        self.sources.changed().map(Response::SourceVolume)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio_client::{Event, NodeInfo};

    const SINK: u32 = 1;
    const SOURCE: u32 = 2;

    fn node(is_sink: bool) -> NodeInfo {
        NodeInfo {
            name: String::new(),
            description: String::new(),
            device_profile_description: String::new(),
            device_id: None,
            card_profile_device: None,
            is_sink,
        }
    }

    fn sink(response: Option<Response>) -> Option<(u32, bool)> {
        match response {
            Some(Response::SinkVolume(v)) => Some((v.volume, v.mute)),
            _ => None,
        }
    }

    fn source(response: Option<Response>) -> Option<(u32, bool)> {
        match response {
            Some(Response::SourceVolume(v)) => Some((v.volume, v.mute)),
            _ => None,
        }
    }

    /// The daemon replays its whole state to every client that subscribes, in
    /// this order (`wmde-settings-daemon/audio-server/src/backend.rs`), at
    /// session start and again after every reconnect. The model reports one
    /// response per direction, once both volume and mute are known; `App`
    /// swallows that first one, so the burst must not produce more.
    #[test]
    fn subscribing_reports_each_direction_once() {
        let mut model = Model::default();
        assert!(model.update(Event::Node(SINK, node(true))).is_none());
        assert!(model.update(Event::Node(SOURCE, node(false))).is_none());
        assert!(model.update(Event::DefaultSink(SINK)).is_none());
        assert!(model.update(Event::DefaultSource(SOURCE)).is_none());
        assert!(model.update(Event::NodeVolume(SINK, 55, None)).is_none());
        assert!(model.update(Event::NodeVolume(SOURCE, 40, None)).is_none());
        assert_eq!(
            sink(model.update(Event::NodeMute(SINK, false))),
            Some((55, false))
        );
        assert_eq!(
            source(model.update(Event::NodeMute(SOURCE, true))),
            Some((40, true))
        );
    }

    fn connected() -> Model {
        let mut model = Model::default();
        for event in [
            Event::Node(SINK, node(true)),
            Event::Node(SOURCE, node(false)),
            Event::DefaultSink(SINK),
            Event::DefaultSource(SOURCE),
            Event::NodeVolume(SINK, 55, None),
            Event::NodeVolume(SOURCE, 40, None),
            Event::NodeMute(SINK, false),
            Event::NodeMute(SOURCE, true),
        ] {
            model.update(event);
        }
        model
    }

    #[test]
    fn volume_key_reports_the_sink() {
        let mut model = connected();
        assert_eq!(
            sink(model.update(Event::NodeVolume(SINK, 60, None))),
            Some((60, false))
        );
    }

    #[test]
    fn microphone_mute_reports_the_source() {
        let mut model = connected();
        assert_eq!(
            source(model.update(Event::NodeMute(SOURCE, false))),
            Some((40, false))
        );
    }

    #[test]
    fn repeating_a_value_stays_quiet() {
        let mut model = connected();
        assert!(model.update(Event::NodeVolume(SINK, 55, None)).is_none());
        assert!(model.update(Event::NodeMute(SINK, false)).is_none());
    }

    #[test]
    fn a_node_that_is_not_default_stays_quiet() {
        let mut model = connected();
        model.update(Event::Node(3, node(true)));
        assert!(model.update(Event::NodeVolume(3, 90, None)).is_none());
        assert!(model.update(Event::NodeMute(3, true)).is_none());
    }

    #[test]
    fn switching_the_default_reports_the_sink() {
        let mut model = connected();
        model.update(Event::Node(3, node(true)));
        model.update(Event::NodeVolume(3, 90, None));
        model.update(Event::NodeMute(3, false));
        assert_eq!(sink(model.update(Event::DefaultSink(3))), Some((90, false)));
    }

    /// A VT switch away and back: the session loses its audio devices, and they
    /// return under new ids with the same volume. Nothing changed for the user.
    #[test]
    fn a_device_that_comes_back_unchanged_stays_quiet() {
        let mut model = connected();
        assert!(model.update(Event::RemoveNode(SOURCE)).is_none());
        assert!(model.update(Event::RemoveNode(SINK)).is_none());
        for event in [
            Event::Node(5, node(true)),
            Event::Node(6, node(false)),
            Event::DefaultSink(5),
            Event::DefaultSource(6),
            Event::NodeVolume(5, 55, None),
            Event::NodeVolume(6, 40, None),
            Event::NodeMute(5, false),
            Event::NodeMute(6, true),
        ] {
            assert!(model.update(event).is_none());
        }
        assert_eq!(
            source(model.update(Event::NodeMute(6, false))),
            Some((40, false))
        );
    }

    #[test]
    fn a_device_that_comes_back_changed_reports_it() {
        let mut model = connected();
        model.update(Event::RemoveNode(SOURCE));
        model.update(Event::Node(6, node(false)));
        model.update(Event::DefaultSource(6));
        model.update(Event::NodeVolume(6, 70, None));
        assert_eq!(
            source(model.update(Event::NodeMute(6, true))),
            Some((70, true))
        );
    }

    #[test]
    fn removing_a_node_keeps_the_active_one() {
        let mut model = connected();
        model.update(Event::Node(3, node(true)));
        model.update(Event::NodeVolume(3, 90, None));
        model.update(Event::NodeMute(3, false));
        model.update(Event::DefaultSink(3));
        model.update(Event::RemoveNode(SINK));
        assert_eq!(
            sink(model.update(Event::NodeVolume(3, 95, None))),
            Some((95, false))
        );
    }
}

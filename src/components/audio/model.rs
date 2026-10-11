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
}

impl Nodes {
    fn active(&self) -> Option<NodeVolume> {
        self.nodes.get(&self.active?)?.value()
    }

    pub fn remove(&mut self, node_id: NodeId) -> bool {
        self.active.take_if(|active| *active == node_id);
        self.nodes.remove(&node_id).is_some()
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
                if let Some(node) = self.sinks.nodes.get_mut(&node_id) {
                    let old_value = node.value();
                    node.mute = Some(mute);
                    let value = node.value();
                    if self.sinks.active == Some(node_id)
                        && old_value != value
                        && let Some(value) = value
                    {
                        return Some(Response::SinkVolume(value));
                    }
                } else if let Some(node) = self.sources.nodes.get_mut(&node_id) {
                    let old_value = node.value();
                    node.mute = Some(mute);
                    let value = node.value();
                    if self.sources.active == Some(node_id)
                        && old_value != value
                        && let Some(value) = value
                    {
                        return Some(Response::SourceVolume(value));
                    }
                }
            }

            audio_client::Event::NodeVolume(node_id, volume, _balance) => {
                let node_id = NodeId(node_id);
                if let Some(node) = self.sinks.nodes.get_mut(&node_id) {
                    let old_value = node.value();
                    node.volume = Some(volume);
                    let value = node.value();
                    if self.sinks.active == Some(node_id)
                        && old_value != value
                        && let Some(value) = value
                    {
                        return Some(Response::SinkVolume(value));
                    }
                } else if let Some(node) = self.sources.nodes.get_mut(&node_id) {
                    let old_value = node.value();
                    node.volume = Some(volume);
                    let value = node.value();
                    if self.sources.active == Some(node_id)
                        && old_value != value
                        && let Some(value) = value
                    {
                        return Some(Response::SourceVolume(value));
                    }
                }
            }

            audio_client::Event::DefaultSink(node_id) => {
                let node_id = NodeId(node_id);
                self.sinks.active = Some(node_id);
                if let Some(value) = self.sinks.active() {
                    return Some(Response::SinkVolume(value));
                }
            }

            audio_client::Event::DefaultSource(node_id) => {
                let node_id = NodeId(node_id);
                self.sources.active = Some(node_id);
                if let Some(value) = self.sources.active() {
                    return Some(Response::SourceVolume(value));
                }
            }

            audio_client::Event::Node(node_id, node) => {
                let node_id = NodeId(node_id);
                if node.is_sink {
                    self.sinks
                        .nodes
                        .entry(node_id)
                        .or_insert_with(Node::default);
                } else {
                    self.sources
                        .nodes
                        .entry(node_id)
                        .or_insert_with(Node::default);
                }
            }

            audio_client::Event::RemoveNode(node_id) => {
                let node_id = NodeId(node_id);
                if !self.sinks.remove(node_id) {
                    self.sources.remove(node_id);
                }
            }

            _ => (),
        }

        None
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

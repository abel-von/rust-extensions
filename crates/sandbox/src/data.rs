use std::collections::HashMap;
use std::time::SystemTime;

use log::warn;
use serde::Deserialize;
use serde::Serialize;
use prost::Message;

use crate::PodSandboxConfig;
use crate::spec::{JsonSpec, Mount, Process};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SandboxData {
    pub id: String,
    pub spec: Option<JsonSpec>,
    pub config: Option<PodSandboxConfig>,
    pub netns: String,
    pub task_address: String,
    pub labels: HashMap<String, String>,
    pub created_at: Option<SystemTime>,
    pub started_at: Option<SystemTime>,
    pub exited_at: Option<SystemTime>,
    pub extensions: HashMap<String, Any>,
}

impl SandboxData {
    pub fn new(req: &crate::api::sandbox::v1::ControllerCreateRequest) -> Self {
        let config = req.options.as_ref().and_then(|x| {
            match PodSandboxConfig::decode(&*x.value) {
                Ok(c) => { Some(c) }
                Err(e) => {
                    warn!("failed to parse container spec {} of {} from request, {}",
                        String::from_utf8_lossy(x.value.as_slice()),req.sandbox_id, e
                    );
                    None
                }
            }
            // match serde_json::from_slice::<PodSandboxConfig>(x.value.as_slice()) {
            //     Ok(s) => Some(s),
            //     Err(e) => {
            //         warn!(
            //             "failed to parse container spec {} of {} from request, {}",
            //             String::from_utf8_lossy(x.value.as_slice()),req.sandbox_id, e
            //         );
            //         None
            //     }
            // }
        });
        Self {
            id: req.sandbox_id.to_string(),
            spec: None,
            config,
            task_address: "".to_string(),
            labels: Default::default(),
            created_at: Some(SystemTime::now()),
            netns: req.netns_path.to_string(),
            started_at: None,
            exited_at: None,
            extensions: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Timestamp {
    pub seconds: i64,
    pub nanos: i32,
}

impl From<prost_types::Timestamp> for Timestamp {
    fn from(from: prost_types::Timestamp) -> Self {
        Self {
            seconds: from.seconds,
            nanos: from.nanos,
        }
    }
}

impl From<Timestamp> for prost_types::Timestamp {
    fn from(from: Timestamp) -> Self {
        Self {
            seconds: from.seconds,
            nanos: from.nanos,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContainerData {
    pub id: String,
    pub spec: Option<JsonSpec>,
    pub rootfs: Vec<Mount>,
    pub io: Option<Io>,
    pub processes: Vec<ProcessData>,
    pub bundle: String,
    pub labels: HashMap<String, String>,
    pub extensions: HashMap<String, Any>,
}

impl ContainerData {
    pub fn new(req: &crate::api::sandbox::v1::PrepareRequest) -> Self {
        let spec = req.spec.as_ref().and_then(|x| {
            match serde_json::from_slice::<JsonSpec>(x.value.as_slice()) {
                Ok(s) => Some(s),
                Err(e) => {
                    warn!(
                        "failed to parse container spec of {} from request, {}",
                        req.container_id, e
                    );
                    None
                }
            }
        });
        Self {
            id: req.container_id.to_string(),
            spec,
            rootfs: req.rootfs.iter().map(Mount::from).collect(),
            io: Some(Io {
                stdin: req.stdin.to_string(),
                stdout: req.stdout.to_string(),
                stderr: req.stderr.to_string(),
                terminal: req.terminal,
            }),
            processes: vec![],
            bundle: "".to_string(),
            labels: Default::default(),
            extensions: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Io {
    pub stdin: String,
    pub stdout: String,
    pub stderr: String,
    pub terminal: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessData {
    pub id: String,
    pub io: Option<Io>,
    pub process: Option<Process>,
    pub extra: HashMap<String, Any>,
}

impl ProcessData {
    pub fn new(req: &crate::api::sandbox::v1::PrepareRequest) -> Self {
        let ps = req
            .spec
            .as_ref()
            .and_then(|x| serde_json::from_slice::<Process>(x.value.as_slice()).ok());
        Self {
            id: req.exec_id.to_string(),
            io: Some(Io {
                stdin: req.stdin.to_string(),
                stdout: req.stdout.to_string(),
                stderr: req.stderr.to_string(),
                terminal: req.terminal,
            }),
            process: ps,
            extra: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Any {
    pub type_url: String,
    pub value: Vec<u8>,
}

impl From<prost_types::Any> for Any {
    fn from(proto: prost_types::Any) -> Self {
        Self {
            type_url: proto.type_url,
            value: proto.value,
        }
    }
}

impl From<Any> for prost_types::Any {
    fn from(any: Any) -> Self {
        Self {
            type_url: any.type_url,
            value: any.value,
        }
    }
}

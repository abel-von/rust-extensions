/*
   Copyright The containerd Authors.

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
*/

use futures::{StreamExt, TryFutureExt};
use std::collections::HashMap;

use lazy_static::lazy_static;
use log::{error, warn};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::Mutex;

use crate::error::Error;
use crate::error::Result;
use crate::monitor::{ExitEvent, Subject, Topic};

lazy_static! {
    pub static ref MONITOR: Mutex<Monitor> = {
        let monitor = Monitor {
            seq_id: 0,
            subscribers: HashMap::new(),
            topic_subs: HashMap::new(),
        };
        Mutex::new(monitor)
    };
}

pub async fn monitor_subscribe(topic: Topic) -> Result<Subscription> {
    let mut monitor = MONITOR.lock().await;
    let s = monitor.subscribe(topic)?;
    Ok(s)
}

pub async fn monitor_notify_by_pid(pid: i32, exit_code: i32) -> Result<()> {
    let monitor = MONITOR.lock().await;
    monitor.notify_by_pid(pid, exit_code).await
}

pub async fn monitor_notify_by_exec(id: &str, exec_id: &str, exit_code: i32) -> Result<()> {
    let monitor = MONITOR.lock().await;
    monitor.notify_by_exec(id, exec_id, exit_code).await
}

pub struct Monitor {
    pub(crate) seq_id: i64,
    pub(crate) subscribers: HashMap<i64, Subscriber>,
    pub(crate) topic_subs: HashMap<Topic, Vec<i64>>,
}

pub(crate) struct Subscriber {
    pub(crate) topic: Topic,
    pub(crate) tx: Sender<ExitEvent>,
}

pub struct Subscription {
    pub id: i64,
    pub rx: Receiver<ExitEvent>,
}

impl Monitor {
    pub fn subscribe(&mut self, topic: Topic) -> Result<Subscription> {
        let (tx, rx) = channel::<ExitEvent>(128);
        let id = self.seq_id;
        self.seq_id += 1;
        let subscriber = Subscriber {
            tx,
            topic: topic.clone(),
        };
        self.subscribers.insert(id, subscriber);
        self.topic_subs
            .entry(topic)
            .or_insert_with(Vec::new)
            .push(id);
        Ok(Subscription { id, rx })
    }

    pub async fn notify_by_pid(&self, pid: i32, exit_code: i32) -> Result<()> {
        let subject = Subject::Pid(pid);
        self.notify_topic(&Topic::Pid, &subject, exit_code).await;
        self.notify_topic(&Topic::All, &subject, exit_code).await;
        Ok(())
    }

    pub async fn notify_by_exec(&self, cid: &str, exec_id: &str, exit_code: i32) -> Result<()> {
        let subject = Subject::Exec(cid.into(), exec_id.into());
        self.notify_topic(&Topic::Exec, &subject, exit_code).await;
        self.notify_topic(&Topic::All, &subject, exit_code).await;
        Ok(())
    }

    // notify_topic try best to notify exit codes to all subscribers and log errors.
    async fn notify_topic(&self, topic: &Topic, subject: &Subject, exit_code: i32) {
        let mut results = Vec::new();
        if let Some(subs) = self.topic_subs.get(topic) {
            while let Some(sub) = subs.iter().filter_map(|x| self.subscribers.get(x)).next() {
                let res = sub
                    .tx
                    .send(ExitEvent {
                        subject: subject.clone(),
                        exit_code,
                    })
                    .await
                    .map_err(other_error!(e, "failed to send exit code"));
                results.push(res);
            }
        }
        while let Some(Err(e)) = results.iter().next() {
            error!("failed to send exit code to subscriber")
        }
    }

    pub fn unsubscribe(&mut self, id: i64) -> Result<()> {
        let sub = self.subscribers.remove(&id);
        if let Some(s) = sub {
            self.topic_subs.get_mut(&s.topic).map(|v| {
                v.iter().position(|&x| x == id).map(|i| {
                    v.remove(i);
                })
            });
        }
        Ok(())
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        // there should be a current handle in the async context
        // it is suggested that the main function be async
        let handle = Handle::current();
        handle.block_on(async move {
            let mut monitor = MONITOR.lock().await;
            monitor.unsubscribe(self.id).unwrap_or_else(|e| {
                error!("failed to unsubscribe the subscription {}, {}", self.id, e);
            });
        });
    }
}

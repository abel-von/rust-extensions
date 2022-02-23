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

mod container;
mod io;
mod runc;
mod service;
mod task;

#[cfg(feature = "async")]
mod asynchronous;

#[cfg(not(feature = "async"))]
fn main() {
    containerd_shim::run::<crate::service::Service>("io.containerd.runc.v2", None)
}

#[cfg(feature = "async")]
#[tokio::main]
async fn main() {
    containerd_shim::run::<Service>("io.containerd.runc.v2", None)
}

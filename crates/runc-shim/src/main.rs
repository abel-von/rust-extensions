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

#[cfg(feature = "async")]
mod asynchronous;
mod common;
#[cfg(not(feature = "async"))]
mod synchronous;

#[cfg(not(feature = "async"))]
fn main() {
    containerd_shim::run::<synchronous::Service>("io.containerd.runc.v2-rs", None)
}

#[cfg(feature = "async")]
fn main() {
    let body = async {
        containerd_shim::asynchronous::run::<crate::asynchronous::Service>(
            "io.containerd.runc.v2-rs",
            None,
        )
        .await;
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed building the runtime");
    runtime.block_on(body);
    runtime.shutdown_background();
}

mod client;
mod prometheus;
mod source;

use nu_plugin::{serve_plugin, JsonSerializer};
use prometheus::Prometheus;

fn main() {
    serve_plugin(&Prometheus, JsonSerializer)
}

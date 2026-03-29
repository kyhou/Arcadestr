// Web entry point: Trunk/WASM build target.

use arcadestr_app::App;
use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();

    mount_to_body(|| view! { <App/> });
}

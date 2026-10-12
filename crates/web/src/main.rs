//! Browser entry point. Built with `trunk`; see the README for how to run it.

use leptos::prelude::*;
use paste2prompt_core::APP_NAME;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

/// Placeholder shell: the app name and an empty canvas area. The real view,
/// tools and controls arrive in later issues.
#[component]
fn App() -> impl IntoView {
    view! {
        <header>
            <h1>{APP_NAME}</h1>
        </header>
        <main>
            <canvas id="stage" width="800" height="500" aria-label="Image canvas (empty)"></canvas>
        </main>
    }
}

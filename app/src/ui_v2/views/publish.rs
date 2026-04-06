use leptos::prelude::*;

use crate::components::PublishView;

#[component]
pub fn PublishV2View() -> impl IntoView {
    view! {
        <section class="v2-publish-wrap">
            <PublishView />
        </section>
    }
}

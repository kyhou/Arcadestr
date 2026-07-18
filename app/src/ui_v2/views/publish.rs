use leptos::prelude::*;

use crate::components::PublishView;

#[component]
pub fn PublishV2View() -> impl IntoView {
    let auth = use_context::<crate::AuthContext>().expect("AuthContext not provided");
    let marketplace = crate::store::use_marketplace_store();
    let selected_game = RwSignal::new(String::new());
    let campaign_id = RwSignal::new(String::new());
    let starts_at = RwSignal::new(String::new());
    let ends_at = RwSignal::new(String::new());
    let current_event_id = RwSignal::new(None::<String>);
    let campaign_root_id = RwSignal::new(None::<String>);
    let update_pointer = RwSignal::new(true);
    let submitting = RwSignal::new(false);
    let result_message = RwSignal::new(None::<String>);

    let publish = move |cancel: bool| {
        let Some(publisher_npub) = auth.npub.get() else {
            result_message.set(Some("Authenticate as the publisher first".into()));
            return;
        };
        let listing_id = selected_game.get();
        let id = campaign_id.get();
        if listing_id.is_empty() || id.trim().is_empty() {
            result_message.set(Some("Select a game and enter a campaign ID".into()));
            return;
        }
        let starts = (!cancel)
            .then(|| starts_at.get().parse::<u64>())
            .transpose();
        let ends = (!cancel).then(|| ends_at.get().parse::<u64>()).transpose();
        let (starts, ends) = match (starts, ends) {
            (Ok(starts), Ok(ends)) => (starts, ends),
            _ => {
                result_message.set(Some("Start and end must be Unix timestamps".into()));
                return;
            }
        };
        submitting.set(true);
        result_message.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            let request = crate::tauri_bridge::PublishCampaignRequest {
                publisher_npub,
                listing_id,
                campaign_id: id,
                starts_at: starts,
                ends_at: ends,
                predecessor_event_id: current_event_id.get(),
                cancel,
                update_listing_pointer: update_pointer.get(),
            };
            match crate::tauri_bridge::invoke_publish_campaign(request).await {
                Ok(response) => {
                    current_event_id.set(Some(response.event_id));
                    campaign_root_id.set(Some(response.root_event_id));
                    result_message.set(Some(if cancel {
                        "Campaign cancelled".into()
                    } else {
                        "Campaign published".into()
                    }));
                }
                Err(error) => result_message.set(Some(error)),
            }
            submitting.set(false);
        });
    };

    view! {
        <section class="v2-publish-wrap">
            <PublishView />
            <section class="v2-panel" style:margin-top="18px" style:padding="16px">
                <h2>"Claim campaign"</h2>
                <p class="v2-social-meta">
                    "Publisher-signed campaign policy. Fulfillment keys cannot use these controls."
                </p>
                <select
                    class="v2-input"
                    on:change:target=move |event| selected_game.set(event.target().value())
                >
                    <option value="">"Select one of your games"</option>
                    {move || auth.npub.get().map(|npub| {
                        marketplace
                            .get_by_publisher(&npub)
                            .into_iter()
                            .map(|game| view! {
                                <option value=game.id.clone()>{game.title}</option>
                            })
                            .collect::<Vec<_>>()
                    }).unwrap_or_default()}
                </select>
                <input
                    class="v2-input"
                    placeholder="Campaign ID"
                    prop:value=move || campaign_id.get()
                    on:input:target=move |event| campaign_id.set(event.target().value())
                />
                <input
                    class="v2-input"
                    placeholder="Start Unix timestamp"
                    prop:value=move || starts_at.get()
                    on:input:target=move |event| starts_at.set(event.target().value())
                />
                <input
                    class="v2-input"
                    placeholder="End Unix timestamp"
                    prop:value=move || ends_at.get()
                    on:input:target=move |event| ends_at.set(event.target().value())
                />
                <label class="v2-social-meta">
                    <input
                        type="checkbox"
                        prop:checked=move || update_pointer.get()
                        on:change:target=move |event| update_pointer.set(event.target().checked())
                    />
                    " Update the listing campaign pointer"
                </label>
                <div style:display="flex" style:gap="8px">
                    <button
                        class="v2-btn-primary"
                        disabled=move || submitting.get()
                        on:click=move |_| publish(false)
                    >
                        {move || if current_event_id.get().is_some() { "Update terms" } else { "Create campaign" }}
                    </button>
                    <button
                        class="v2-btn-secondary"
                        disabled=move || submitting.get() || current_event_id.get().is_none()
                        on:click=move |_| publish(true)
                    >
                        "Cancel campaign"
                    </button>
                </div>
                {move || campaign_root_id.get().map(|root| view! {
                    <p class="v2-social-meta">{format!("Campaign root: {root}")}</p>
                })}
                {move || result_message.get().map(|message| view! {
                    <p class="v2-social-meta">{message}</p>
                })}
            </section>
        </section>
    }
}

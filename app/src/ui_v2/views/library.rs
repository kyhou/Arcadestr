use leptos::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::tauri_bridge::{invoke_get_installed_games, InstalledGame};

fn render_installed_game_card(game: InstalledGame) -> AnyView {
    let version = game.version.unwrap_or_else(|| "unknown".to_string());

    view! {
        <article class="v2-library-media-card">
            <div class="v2-library-media-copy">
                <h4>{game.game_coordinate}</h4>
                <p class="v2-social-meta">{format!("Version: {version}")}</p>
                <p class="v2-social-meta">{format!("Server: {}", game.server_url)}</p>
                <p class="v2-social-meta" style:word-break="break-all">
                    {format!("File hash: {}", game.file_hash)}
                </p>
                <p class="v2-social-meta" style:word-break="break-all">
                    {format!("Local path: {}", game.file_path)}
                </p>
                <button class="v2-btn-ghost" disabled=true>"Installed"</button>
            </div>
        </article>
    }
    .into_any()
}

#[component]
pub fn LibraryView() -> impl IntoView {
    let installed_games = RwSignal::new(Vec::<InstalledGame>::new());
    let library_loading = RwSignal::new(true);
    let library_error = RwSignal::new(None::<String>);

    Effect::new(move |_| {
        spawn_local(async move {
            library_loading.set(true);
            library_error.set(None);
            match invoke_get_installed_games().await {
                Ok(games) => installed_games.set(games),
                Err(err) => library_error.set(Some(err)),
            }
            library_loading.set(false);
        });
    });

    view! {
        <section class="v2-library-grid">
            <header class="v2-panel-glass v2-library-hero">
                <div>
                    <h1 class="v2-display">"My Library"</h1>
                    <p>"Installed ADP games recorded on this device."</p>
                </div>
                <div class="v2-tab-row v2-library-tabs">
                    <button class="v2-tab active">"Installed"</button>
                </div>
            </header>

            <div class="v2-library-layout-grid">
                <section class="v2-library-main-grid">
                    {move || {
                        if library_loading.get() {
                            view! {
                                <article class="v2-panel-glass v2-library-media-copy">
                                    <h3>"Loading installed games..."</h3>
                                    <p class="v2-social-meta">"Checking the local ADP install registry."</p>
                                </article>
                            }.into_any()
                        } else if let Some(err) = library_error.get() {
                            view! {
                                <article class="v2-panel-glass v2-library-media-copy">
                                    <h3>"Could not load installed games"</h3>
                                    <p class="v2-social-meta" style:color="var(--v2-danger)">{err}</p>
                                </article>
                            }.into_any()
                        } else {
                            let games = installed_games.get();
                            if games.is_empty() {
                                view! {
                                    <article class="v2-panel-glass v2-library-media-copy">
                                        <h3>"No installed games"</h3>
                                        <p class="v2-social-meta">"No installed games yet. Buy and install a game to see it here."</p>
                                    </article>
                                }.into_any()
                            } else {
                                games.into_iter().map(render_installed_game_card).collect::<Vec<_>>().into_any()
                            }
                        }
                    }}
                </section>

                <aside class="v2-library-side-grid">
                    <section class="v2-panel-glass v2-identity-card">
                        <h3>"Library Status"</h3>
                        <div class="v2-stat-line">
                            <span>"Installed Games"</span>
                            <strong>{move || installed_games.get().len().to_string()}</strong>
                        </div>
                        <p class="v2-social-meta">"Runtime execution and extraction are intentionally out of scope for this gate."</p>
                    </section>

                    <section class="v2-panel-glass v2-notes-card">
                        <h3>"ADP Inventory"</h3>
                        <p class="v2-social-meta">"This list is loaded from the local installed-games registry."</p>
                    </section>
                </aside>
            </div>
        </section>
    }
}

use leptos::prelude::*;

#[component]
pub fn LibraryView() -> impl IntoView {
    view! {
        <section class="v2-library-grid">
            <header class="v2-panel-glass v2-library-hero">
                <div>
                    <h1 class="v2-display">"My Library"</h1>
                    <p>"Decentralized game ownership via npub1...8qz9"</p>
                </div>
                <div class="v2-tab-row v2-library-tabs">
                    <button class="v2-tab active">"Recent"</button>
                    <button class="v2-tab">"Installed"</button>
                    <button class="v2-tab">"Favorites"</button>
                </div>
            </header>

            <div class="v2-library-layout-grid">
                <section class="v2-library-main-grid">
                    <article class="v2-library-feature-card">
                        <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuDff3J3x4fIT20MDOhaAm_PXnkQYHlmUbBAh9aNHRH2o0xkrV-aAlW1IFSBmEUBw_eupAOV0wu-yb393NRh7o74JeWsdpvX5l795h1j9UiyQWWljsQ6pTbdjfApVODvmD4_u3LvyUR7GgiPNeidZkR2BUixem3S0gTrGB9FZuDzpRPyHF3Z8GK817ZWl2PfPGqjIy9puKYrqZ5Y81Hh0cjRa5ny5sa9-6l6TNnqFWYs1l_TcbwFvOwvpBz5gCb_Wx8ZyR450U1ussY" alt="Cyber Strata" />
                        <div class="v2-library-feature-overlay">
                            <div>
                                <h2 class="v2-display">"Cyber Strata"</h2>
                                <p class="v2-social-meta">"Last played: 2 hours ago"</p>
                            </div>
                            <button class="v2-btn-primary">"Play"</button>
                        </div>
                    </article>

                    <article class="v2-library-media-card">
                        <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuBcd5y4llmLMyWVXXFwN5Mc7PGf1OooEwPPgVU_1D9cFz7fAFVkv1jkLkKjO5LVM8WjF4YijWHIRNollS3LxFkCglLAtKiZrZwCKbfN4Upo7ZSBGaZsH18oT7abCNWgntIo2khXAiV4lKM6Ra87y8S9Y7aQ04JF_oTNpu9z4hQLF2i06AJSDU7N2GR8rY5JpFoJl7z_vnbdrAIK2Fdwnvs0jgJrAQF_YLhI4CZ8XIc0Rh0wds9n6tKSSkVc61xPdzR8m78nDW9vYqI" alt="Pixel Void" />
                        <div class="v2-library-media-copy">
                            <h4>"Pixel Void"</h4>
                            <p class="v2-social-meta">"⚡ 1.1k Zaps"</p>
                            <button class="v2-btn-ghost">"Install"</button>
                        </div>
                    </article>

                    <article class="v2-library-media-card">
                        <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuBxQHPVGNGBjPOsjaFMeYFlUSvPXbDVtwFPotfQpIOxV2sSayc4wmV3wuW1bB5Yw0oHe3I4h5zAr2icmvobs-csqhRdAUat8iEjw7BiXUgeJgaGJacx0vuLHFnRLMSPKk1aMZExWs9IOQ6LVen-R_dRlPia537LU5U4wmsRMikMsfk4BnyoE5t3Z15idW00_a0nozWQp-NBVYnFCHelNbGFmIrDGSGc0IuuXqbAYKhE3dmxdjP-ZSCP77U0NCXMLATezsD19tmbZxI" alt="Nebula Drifter" />
                        <div class="v2-library-media-copy">
                            <h4>"Nebula Drifter"</h4>
                            <p class="v2-social-meta">"⚡ 842 Zaps"</p>
                            <button class="v2-btn-ghost">"Launch"</button>
                        </div>
                    </article>

                    <article class="v2-library-media-card">
                        <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuBn_-keHwH2sFgySSNoCIBNundLkGLsXZ1AmPgvxGn4X2vHKrWgl77eBFrGscOKd7ZZh1wbNKMwzLMW4nCOdvE7te8sPdeheiYXx9tASWcbmLTQ58T581ZN8LIx2p2ygNHcSc0xxqp7zAurg4PVMJUWLE2nXuE4LDjx5Wn3AqAspHChopHWosPbApPmBMOA0_XdvzHnQLhPj2yzIn6lYKm0crm7VZfmBq8QwgEAiVAjJKkzfkih1YysXuc1EK-_fTynOeXEhgiAiVo" alt="Rogue Protocol" />
                        <div class="v2-library-media-copy">
                            <h4>"Rogue Protocol"</h4>
                            <p class="v2-social-meta">"⚡ 3.2k Zaps"</p>
                            <button class="v2-btn-ghost">"Launch"</button>
                        </div>
                    </article>
                </section>

                <aside class="v2-library-side-grid">
                    <section class="v2-panel-glass v2-identity-card">
                        <h3>"User Identity"</h3>
                        <div class="v2-stat-line"><span>"Zaps Given"</span><strong>"12.8k"</strong></div>
                        <div class="v2-stat-line"><span>"Zaps Received"</span><strong>"4.2k"</strong></div>
                        <div class="v2-stat-line"><span>"Friends Online"</span><strong>"14"</strong></div>
                        <button class="v2-btn-secondary v2-identity-connect">"Connect Nostr"</button>
                    </section>

                    <section class="v2-panel-glass v2-notes-card">
                        <h3>"Recent Community Notes"</h3>
                        <div class="v2-live-note">
                            <p class="v2-social-meta">"SatStacker - 4m"</p>
                            <p>"New boss fight in Cyber Strata is intense."</p>
                        </div>
                        <div class="v2-live-note">
                            <p class="v2-social-meta">"BitRunner - 22m"</p>
                            <p>"Anyone down for co-op in Nebula Drifter?"</p>
                        </div>
                    </section>

                    <section class="v2-panel v2-friends-card">
                        <h3>"Friends List"</h3>
                        <div class="v2-friends-row">
                            <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuDniAjIK9VWl3rBqLP-BR8p_eK1wurYJSy8wCWUA_1w95Bt_fwV3rwinvjd3_2QcIpb8_xeIblFWIC-nyr2MHrI9BxDVS9HUjUUJzU2_lh80p82MogPUlcTVmRE0VAz7vpDJ-GZZYCdh8KbS4SALsVjx32yfGr4x4mXyUOCh6rMjGDCCfhyp1aikQ0XSWCPKHHO4hhuygQYqNfKq9fyYtz546UAX-67rwa5EcNRfW0W5e1uOsaA9iZZF_gHPbPyCsY-nDFErgppROI" alt="friend" />
                            <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuBcjEeYZ1RTfEEI6Ua-ImE6RVRRSbIkPB9W7VnfOGHf2x-JVF_n8RgS5QAiAjrt4kuccBLdXy8Sfue_gd1ImQS6NEYlTRvVy005sCSUQPs2ZKZqnWO0ipnQVPlSTneenyC42TLtv8KUjDtzFCEF8G_k8EHpLn0Yewv6sv7sqpPBr7LslUlBiGawf2LLaGz5pIPyD2XLFIi3OwiStUpRti8qvJkyYuXfo6M1wvKYKkQvMmCjUMet9SavgwSVyMegIF1NgCO3kzL3hUI" alt="friend" />
                            <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuBuvn9geI_jl_Wt2rPRoyCWOfUv1IugsvhMlYIgDUIf8SYfKoKNZ8Gz9jHTcLuncbCJw2-DSqROr6bG1KnqUyZeqa4_lMugvgwlYPDLlnk9uVFj9BSUP5DDFBeVDIhGqWBZerNsv5o8CJ5xmD-9A2CCLTSPO3I5QtGuFyBNhDuw6sF-Fz_AB1XOGOCogP6e7iWg-9ODkNknCCK7vIsPIBaxrI6rBEg1z5jPUM4a4GrrSYZbJ8whsnlM5rsaBZmt-YRPsDh0ZQwVOWU" alt="friend" />
                            <img src="https://lh3.googleusercontent.com/aida-public/AB6AXuAi0xNe17tHCYOqd3EP6XA8eUZSZRj2zhIkmWOIZu9bc-E7WNjCRC2KmXGoQM8H8wSdSeqs5d30tDG0wMsDyzjB713jz8t7fm2Mfgsu8z8VEtExDjZkK7KI5xr2p4mFCbfutLsXsdCJjDTVLSG2rz7kOn6ZI4PrxBSDR_RADqp3OF7EmfGeM0j13XRzhqEMnFZ7KHsrL90l6AqCf3CM04m7KA_B8cplufiW-QYkMVfWaOI-q9D4Qc4ZVpSBO82mVCDMT1PQqHL8gWw" alt="friend" />
                            <button class="v2-btn-ghost">"+"</button>
                        </div>
                    </section>
                </aside>
            </div>
        </section>
    }
}

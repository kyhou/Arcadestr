use leptos::prelude::*;
use time::{Date, Month};

#[derive(Clone, Copy, PartialEq)]
enum SelectionTarget {
    Start,
    End,
}

#[component]
pub fn DateTimeRangePicker(
    #[prop(into)] starts_at: Signal<String>,
    #[prop(into)] ends_at: Signal<String>,
    on_starts_at: Callback<String>,
    on_ends_at: Callback<String>,
    #[prop(into)] disabled: Signal<bool>,
) -> impl IntoView {
    let initial_date = date_from_datetime(&starts_at.get_untracked())
        .or_else(current_local_date)
        .unwrap_or(Date::MIN);
    let display_date = RwSignal::new(initial_date);
    let selection_target = RwSignal::new(SelectionTarget::Start);

    let previous_month = move |_| {
        display_date.update(|date| *date = adjacent_month(*date, false));
    };
    let next_month = move |_| {
        display_date.update(|date| *date = adjacent_month(*date, true));
    };

    view! {
        <div class="w-full max-w-[22rem] overflow-hidden rounded-xl border border-outline-variant/25 bg-surface-container-highest shadow-ambient">
            <div class="grid grid-cols-2 gap-px bg-outline-variant/20">
                <button
                    type="button"
                    class=move || selection_button_class(selection_target.get() == SelectionTarget::Start)
                    disabled=move || disabled.get()
                    on:click=move |_| {
                        selection_target.set(SelectionTarget::Start);
                        if let Some(date) = date_from_datetime(&starts_at.get_untracked()) {
                            display_date.set(date);
                        }
                    }
                >
                    <span class="text-[10px] font-bold uppercase tracking-widest text-secondary">"Starts"</span>
                    <strong class="truncate text-xs">{move || display_datetime(&starts_at.get())}</strong>
                </button>
                <button
                    type="button"
                    class=move || selection_button_class(selection_target.get() == SelectionTarget::End)
                    disabled=move || disabled.get()
                    on:click=move |_| {
                        selection_target.set(SelectionTarget::End);
                        if let Some(date) = date_from_datetime(&ends_at.get_untracked()) {
                            display_date.set(date);
                        }
                    }
                >
                    <span class="text-[10px] font-bold uppercase tracking-widest text-secondary">"Ends"</span>
                    <strong class="truncate text-xs">{move || display_datetime(&ends_at.get())}</strong>
                </button>
            </div>

            <div class="p-3">
                <header class="mb-2 grid grid-cols-[2rem_1fr_2rem] items-center">
                    <button type="button" class="inline-flex h-8 w-8 items-center justify-center rounded-lg text-on-surface-variant transition-colors hover:bg-surface-bright hover:text-on-surface disabled:opacity-40" aria-label="Previous month" disabled=move || disabled.get() on:click=previous_month>
                        <span class="material-symbols-outlined text-lg">"chevron_left"</span>
                    </button>
                    <strong class="text-center text-sm font-bold">{move || format!("{} {}", display_date.get().month(), display_date.get().year())}</strong>
                    <button type="button" class="inline-flex h-8 w-8 items-center justify-center rounded-lg text-on-surface-variant transition-colors hover:bg-surface-bright hover:text-on-surface disabled:opacity-40" aria-label="Next month" disabled=move || disabled.get() on:click=next_month>
                        <span class="material-symbols-outlined text-lg">"chevron_right"</span>
                    </button>
                </header>

                <div class="grid grid-cols-7 text-center text-[11px] font-semibold text-on-surface-variant" aria-hidden="true">
                    <span>"Mo"</span><span>"Tu"</span><span>"We"</span><span>"Th"</span><span>"Fr"</span><span>"Sa"</span><span>"Su"</span>
                </div>
                <div class="mt-1 grid grid-cols-7 justify-items-center gap-1" role="grid" aria-label="Choose a date">
                    {move || {
                        let year = display_date.get().year();
                        let month = display_date.get().month();
                        calendar_days(year, month)
                            .into_iter()
                            .enumerate()
                            .map(|(index, day)| {
                                let date = day.and_then(|day| Date::from_calendar_date(year, month, day).ok());
                                match date {
                                    Some(date) => view! {
                                        <button
                                            type="button"
                                            role="gridcell"
                                            class=move || calendar_day_class(
                                                date,
                                                date_from_datetime(&starts_at.get()),
                                                date_from_datetime(&ends_at.get()),
                                            )
                                            aria-label=format!("{} {date}", if selection_target.get_untracked() == SelectionTarget::Start { "Set start to" } else { "Set end to" })
                                            disabled=move || disabled.get()
                                            on:click=move |_| {
                                                let formatted = format_date(date);
                                                match selection_target.get_untracked() {
                                                    SelectionTarget::Start => {
                                                        on_starts_at.run(replace_datetime_date(&starts_at.get_untracked(), &formatted));
                                                        selection_target.set(SelectionTarget::End);
                                                    }
                                                    SelectionTarget::End => {
                                                        on_ends_at.run(replace_datetime_date(&ends_at.get_untracked(), &formatted));
                                                        selection_target.set(SelectionTarget::Start);
                                                    }
                                                }
                                            }
                                        >{date.day()}</button>
                                    }.into_any(),
                                    None => view! { <span role="gridcell" aria-hidden="true" data-index=index></span> }.into_any(),
                                }
                            })
                            .collect_view()
                    }}
                </div>
            </div>

            <div class="grid grid-cols-2 gap-2 border-t border-outline-variant/20 p-3">
                <label class="grid min-w-0 gap-1 text-xs font-semibold text-on-surface-variant">
                    <span>"Start time"</span>
                    <span class="relative">
                        <input aria-label="Start time" required=true class="w-full min-w-0 rounded-lg border border-outline-variant/25 bg-surface-container-high py-2 pl-2.5 pr-8 text-sm text-on-surface [&::-webkit-calendar-picker-indicator]:hidden" type="time" step="60" prop:value=move || datetime_time(&starts_at.get()) on:input:target=move |event| on_starts_at.run(replace_datetime_time(&starts_at.get_untracked(), &event.target().value())) disabled=move || disabled.get() />
                        <span class="material-symbols-outlined pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-lg text-on-surface-variant">"schedule"</span>
                    </span>
                </label>
                <label class="grid min-w-0 gap-1 text-xs font-semibold text-on-surface-variant">
                    <span>"End time"</span>
                    <span class="relative">
                        <input aria-label="End time" required=true class="w-full min-w-0 rounded-lg border border-outline-variant/25 bg-surface-container-high py-2 pl-2.5 pr-8 text-sm text-on-surface [&::-webkit-calendar-picker-indicator]:hidden" type="time" step="60" prop:value=move || datetime_time(&ends_at.get()) on:input:target=move |event| on_ends_at.run(replace_datetime_time(&ends_at.get_untracked(), &event.target().value())) disabled=move || disabled.get() />
                        <span class="material-symbols-outlined pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 text-lg text-on-surface-variant">"schedule"</span>
                    </span>
                </label>
            </div>
        </div>
    }
}

#[component]
pub fn DatePicker(
    #[prop(into)] value: Signal<String>,
    on_value: Callback<String>,
    #[prop(into)] disabled: Signal<bool>,
) -> impl IntoView {
    let initial_date = date_from_datetime(&value.get_untracked())
        .or_else(current_local_date)
        .unwrap_or(Date::MIN);
    let display_date = RwSignal::new(initial_date);
    let open = RwSignal::new(false);

    view! {
        <div class="relative">
            <button
                type="button"
                class="v2-input flex min-h-12 items-center justify-between gap-3 text-left"
                aria-label="Choose release date"
                aria-expanded=move || open.get()
                disabled=move || disabled.get()
                on:click=move |_| {
                    if let Some(date) = date_from_datetime(&value.get_untracked()) {
                        display_date.set(date);
                    }
                    open.update(|open| *open = !*open);
                }
            >
                <span>{move || {
                    let current = value.get();
                    if current.trim().is_empty() { "Select a date".to_string() } else { current }
                }}</span>
                <span class="material-symbols-outlined text-lg text-on-surface-variant" aria-hidden="true">"calendar_month"</span>
            </button>
            <Show when=move || open.get()>
                <section class="absolute left-0 top-[calc(100%+0.5rem)] z-50 w-[min(22rem,calc(100vw-3rem))] rounded-xl border border-outline-variant/40 bg-surface-container-highest p-3 shadow-ambient" aria-label="Release date calendar">
                    <header class="mb-2 grid grid-cols-[2rem_1fr_2rem] items-center">
                        <button type="button" class="inline-flex h-8 w-8 items-center justify-center rounded-lg text-on-surface-variant hover:bg-surface-bright hover:text-on-surface" aria-label="Previous month" on:click=move |_| display_date.update(|date| *date = adjacent_month(*date, false))>
                            <span class="material-symbols-outlined text-lg">"chevron_left"</span>
                        </button>
                        <strong class="text-center text-sm font-bold">{move || format!("{} {}", display_date.get().month(), display_date.get().year())}</strong>
                        <button type="button" class="inline-flex h-8 w-8 items-center justify-center rounded-lg text-on-surface-variant hover:bg-surface-bright hover:text-on-surface" aria-label="Next month" on:click=move |_| display_date.update(|date| *date = adjacent_month(*date, true))>
                            <span class="material-symbols-outlined text-lg">"chevron_right"</span>
                        </button>
                    </header>
                    <div class="grid grid-cols-7 text-center text-[11px] font-semibold text-on-surface-variant" aria-hidden="true">
                        <span>"Mo"</span><span>"Tu"</span><span>"We"</span><span>"Th"</span><span>"Fr"</span><span>"Sa"</span><span>"Su"</span>
                    </div>
                    <div class="mt-1 grid grid-cols-7 justify-items-center gap-1" role="grid" aria-label="Choose release date">
                        {move || {
                            let year = display_date.get().year();
                            let month = display_date.get().month();
                            calendar_days(year, month)
                                .into_iter()
                                .enumerate()
                                .map(|(index, day)| {
                                    let date = day.and_then(|day| Date::from_calendar_date(year, month, day).ok());
                                    match date {
                                        Some(date) => view! {
                                            <button
                                                type="button"
                                                role="gridcell"
                                                class=move || calendar_day_class(date, date_from_datetime(&value.get()), None)
                                                aria-label=format!("Set release date to {date}")
                                                on:click=move |_| {
                                                    on_value.run(format_date(date));
                                                    open.set(false);
                                                }
                                            >{date.day()}</button>
                                        }.into_any(),
                                        None => view! { <span role="gridcell" aria-hidden="true" data-index=index></span> }.into_any(),
                                    }
                                })
                                .collect_view()
                        }}
                    </div>
                    <div class="mt-3 flex justify-end border-t border-outline-variant/20 pt-3">
                        <button type="button" class="v2-btn-secondary" on:click=move |_| { on_value.run(String::new()); open.set(false); }>"Clear date"</button>
                    </div>
                </section>
            </Show>
        </div>
    }
}

fn selection_button_class(selected: bool) -> &'static str {
    if selected {
        "grid min-w-0 gap-0.5 bg-surface-container-high px-3 py-2.5 text-left ring-1 ring-inset ring-primary disabled:opacity-50"
    } else {
        "grid min-w-0 gap-0.5 bg-surface-container px-3 py-2.5 text-left text-on-surface-variant transition-colors hover:bg-surface-container-high disabled:opacity-50"
    }
}

fn calendar_day_class(date: Date, start: Option<Date>, end: Option<Date>) -> &'static str {
    if start == Some(date) || end == Some(date) {
        "h-9 w-9 rounded-lg bg-primary text-sm font-bold text-on-primary shadow-glow-primary disabled:opacity-50"
    } else if start.is_some_and(|start| date > start) && end.is_some_and(|end| date < end) {
        "h-9 w-9 rounded-lg bg-primary/15 text-sm font-semibold text-primary disabled:opacity-50"
    } else {
        "h-9 w-9 rounded-lg text-sm text-on-surface transition-colors hover:bg-surface-bright disabled:opacity-50"
    }
}

fn calendar_days(year: i32, month: Month) -> Vec<Option<u8>> {
    let Some(first_day) = Date::from_calendar_date(year, month, 1).ok() else {
        return Vec::new();
    };
    let leading = first_day.weekday().number_from_monday() as usize - 1;
    let days_in_month = month.length(year);
    let occupied = leading + usize::from(days_in_month);
    let trailing = (7 - occupied % 7) % 7;
    let mut days = Vec::with_capacity(occupied + trailing);
    days.extend((0..leading).map(|_| None));
    days.extend((1..=days_in_month).map(Some));
    days.extend((0..trailing).map(|_| None));
    days
}

fn adjacent_month(date: Date, next: bool) -> Date {
    let (year, month) = match (next, date.month()) {
        (true, Month::December) => (date.year() + 1, Month::January),
        (true, month) => (date.year(), month.next()),
        (false, Month::January) => (date.year() - 1, Month::December),
        (false, month) => (date.year(), month.previous()),
    };
    Date::from_calendar_date(year, month, 1).unwrap_or(date)
}

fn current_local_date() -> Option<Date> {
    let date = js_sys::Date::new_0();
    Date::from_calendar_date(
        date.get_full_year() as i32,
        Month::try_from((date.get_month() + 1) as u8).ok()?,
        date.get_date() as u8,
    )
    .ok()
}

fn date_from_datetime(value: &str) -> Option<Date> {
    let date = value.split_once('T').map_or(value, |(date, _)| date);
    let mut parts = date.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = Month::try_from(parts.next()?.parse::<u8>().ok()?).ok()?;
    let day = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Date::from_calendar_date(year, month, day).ok()
}

fn format_date(date: Date) -> String {
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        u8::from(date.month()),
        date.day()
    )
}

fn datetime_time(value: &str) -> String {
    let time = value.split_once('T').map_or("", |(_, time)| time);
    let mut parts = time.split(':');
    match (parts.next(), parts.next()) {
        (Some(hour), Some(minute)) => format!("{hour}:{minute}"),
        _ => time.to_string(),
    }
}

fn replace_datetime_date(current: &str, date: &str) -> String {
    format!("{date}T{}", datetime_time(current))
}

fn replace_datetime_time(current: &str, time: &str) -> String {
    let date = current.split_once('T').map_or(current, |(date, _)| date);
    format!("{date}T{time}")
}

fn display_datetime(value: &str) -> String {
    let date = value.split_once('T').map_or(value, |(date, _)| date);
    let time = datetime_time(value);
    if time.is_empty() {
        date.to_string()
    } else {
        format!("{date} · {time}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn february_calendar_is_monday_aligned_and_complete() {
        let days = calendar_days(2024, Month::February);

        assert_eq!(days.len(), 35);
        assert_eq!(&days[..3], &[None, None, None]);
        assert_eq!(days[3], Some(1));
        assert_eq!(days[31], Some(29));
    }

    #[test]
    fn datetime_updates_preserve_the_other_value() {
        let value = "2026-07-26T14:45:30";

        assert_eq!(datetime_time(value), "14:45");
        assert_eq!(
            replace_datetime_date(value, "2026-08-01"),
            "2026-08-01T14:45"
        );
        assert_eq!(replace_datetime_time(value, "09:30"), "2026-07-26T09:30");
    }
}

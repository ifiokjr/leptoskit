// The benchmark scales this file up by repeating it with every `__N__`
// replaced by a number, so it reads like one large module of components.

#[derive(Clone, Debug, PartialEq)]
pub struct Metric__N__ {
	pub id: u32,
	pub label: String,
	pub value: f64,
	pub trend: f64,
}

#[component]
pub fn Dashboard__N__(metrics: ReadSignal<Vec<Metric__N__>>, user: Signal<Option<User>>) -> impl IntoView {
	let (filter, set_filter) = signal(String::new());
	let (selected, set_selected) = signal(None::<u32>);
	let visible = move || {
		metrics.with(|metrics| {
			metrics
				.iter()
				.filter(|metric| metric.label.to_lowercase().contains(&filter.get().to_lowercase()))
				.cloned()
				.collect::<Vec<_>>()
		})
	};
	view! {
		<div class="dashboard flex min-h-screen flex-col bg-slate-50 text-slate-900">
			<header class="flex items-center justify-between border-b border-slate-200 px-6 py-4">
				<h1 class="text-2xl font-semibold tracking-tight">"Overview"</h1>
				<nav class="flex gap-4" aria-label="Sections">
					<a href="/" class="text-slate-600 hover:text-slate-900">"Home"</a>
					<a href="/reports" class="text-slate-600 hover:text-slate-900">"Reports"</a>
					<a href="/settings" class="text-slate-600 hover:text-slate-900">"Settings"</a>
				</nav>
				<Show when=move || user.get().is_some() fallback=|| view! { <SignIn__N__ /> }>
					<UserMenu__N__ user=user />
				</Show>
			</header>
			<main class="grid flex-1 grid-cols-1 gap-6 p-6 lg:grid-cols-3">
				<section class="lg:col-span-2">
					<div class="mb-4 flex items-center gap-2">
						<label for="filter" class="sr-only">"Filter metrics"</label>
						<input
							id="filter"
							type="search"
							class="w-full rounded-md border border-slate-300 px-3 py-2"
							placeholder="Filter metrics"
							prop:value=filter
							on:input=move |ev| set_filter.set(event_target_value(&ev))
						/>
						<button class="rounded-md bg-slate-900 px-3 py-2 text-white" on:click=move |_| set_filter.set(String::new())>
							"Clear"
						</button>
					</div>
					<ul class="divide-y divide-slate-200 rounded-lg border border-slate-200 bg-white">
						<For each=visible key=|metric| metric.id let:metric>
							<li
								class="flex items-center justify-between px-4 py-3"
								class:bg-slate-100=move || selected.get() == Some(metric.id)
								on:click=move |_| set_selected.set(Some(metric.id))
							>
								<span class="font-medium">{metric.label.clone()}</span>
								<span class="tabular-nums">{format!("{:.2}", metric.value)}</span>
								<Trend__N__ value=metric.trend />
							</li>
						</For>
					</ul>
				</section>
				<aside class="space-y-4">
					<Card__N__ title="Selected">
						{move || match selected.get() {
							Some(id) => view! { <MetricDetail__N__ id=id on_close=move || set_selected.set(None) /> }.into_any(),
							None => view! { <p class="text-slate-500">"Select a metric to see its history."</p> }.into_any(),
						}}
					</Card__N__>
					<Card__N__ title="Summary">
						<dl class="grid grid-cols-2 gap-2 text-sm">
							<dt class="text-slate-500">"Metrics"</dt>
							<dd class="text-right tabular-nums">{move || metrics.with(Vec::len)}</dd>
							<dt class="text-slate-500">"Visible"</dt>
							<dd class="text-right tabular-nums">{move || visible().len()}</dd>
							<dt class="text-slate-500">"Rising"</dt>
							<dd class="text-right tabular-nums">
								{move || metrics.with(|metrics| metrics.iter().filter(|metric| metric.trend > 0.0).count())}
							</dd>
						</dl>
					</Card__N__>
				</aside>
			</main>
			<footer class="border-t border-slate-200 px-6 py-3 text-sm text-slate-500">
				"Updated " {move || format_relative(now())} " · " <a href="/changelog" class="underline">"What changed"</a>
			</footer>
		</div>
	}
}

#[component]
fn Card__N__(title: &'static str, children: Children) -> impl IntoView {
	view! {
		<div class="rounded-lg border border-slate-200 bg-white p-4 shadow-sm">
			<h2 class="mb-2 text-sm font-semibold uppercase tracking-wide text-slate-500">{title}</h2>
			{children()}
		</div>
	}
}

#[component]
fn Trend__N__(value: f64) -> impl IntoView {
	let (icon, class) = if value > 0.0 { ("▲", "text-emerald-600") } else if value < 0.0 { ("▼", "text-rose-600") } else { ("■", "text-slate-400") };
	view! { <span class=format!("text-xs {class}") aria-label=format!("trend {value:.1}%")>{icon}" "{format!("{:.1}%", value.abs())}</span> }
}

#[component]
fn MetricDetail__N__(id: u32, on_close: impl Fn() + 'static) -> impl IntoView {
	let history = Resource::new(move || id, |id| async move { fetch_history(id).await });
	view! {
		<div class="space-y-2">
			<div class="flex items-center justify-between">
				<h3 class="font-medium">"Metric #"{id}</h3>
				<button class="text-slate-500 hover:text-slate-900" aria-label="Close" on:click=move |_| on_close()>"×"</button>
			</div>
			<Suspense fallback=|| view! { <p class="animate-pulse text-slate-400">"Loading history…"</p> }>
				{move || Suspend::new(async move {
					let points = history.await;
					view! {
						<svg viewBox="0 0 100 30" class="h-16 w-full">
							<polyline fill="none" stroke="currentColor" stroke-width="1.5" points=sparkline(&points) />
						</svg>
						<p class="text-xs text-slate-500">{points.len()}" points"</p>
					}
				})}
			</Suspense>
		</div>
	}
}

#[component]
fn SignIn__N__() -> impl IntoView {
	view! {
		<form class="flex gap-2" method="post" action="/sign-in">
			<input type="email" name="email" required placeholder="you@example.com" class="rounded-md border px-2 py-1" />
			<button type="submit" class="rounded-md bg-slate-900 px-3 py-1 text-white">"Sign in"</button>
		</form>
	}
}

#[component]
fn UserMenu__N__(user: Signal<Option<User>>) -> impl IntoView {
	let (open, set_open) = signal(false);
	view! {
		<div class="relative">
			<button class="flex items-center gap-2" aria-haspopup="menu" aria-expanded=move || open.get().to_string() on:click=move |_| set_open.update(|open| *open = !*open)>
				<img class="h-8 w-8 rounded-full" src=move || user.get().map(|user| user.avatar).unwrap_or_default() alt="" />
				<span class="hidden sm:inline">{move || user.get().map(|user| user.name).unwrap_or_default()}</span>
			</button>
			<Show when=move || open.get()>
				<ul role="menu" class="absolute right-0 mt-2 w-48 rounded-md border bg-white py-1 shadow-lg">
					<li role="none"><a role="menuitem" href="/profile" class="block px-4 py-2 hover:bg-slate-100">"Profile"</a></li>
					<li role="none"><a role="menuitem" href="/sign-out" class="block px-4 py-2 hover:bg-slate-100">"Sign out"</a></li>
				</ul>
			</Show>
		</div>
	}
}

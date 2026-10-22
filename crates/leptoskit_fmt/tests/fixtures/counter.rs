use leptos::prelude::*;

/// A counter.
#[component]
pub fn Counter(initial: i32) -> impl IntoView {
  let (count,set_count)=signal(initial);
    view!{
  <div class="counter">
      <button on:click=move|_|set_count.update(|n|*n-=1)>"-"</button>
         <span>{count}</span>
      <button on:click=move |_| set_count.update(|n| *n += 1)>
      "+"
      </button>
    </div>
    }
}

#[component]
pub fn App() -> impl IntoView {
view! {    <Counter initial=3/>   }
}

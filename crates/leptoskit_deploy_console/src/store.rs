use reactive_graph::signal::ArcRwSignal;
use reactive_graph::traits::Update;
use reactive_graph::traits::With;

use crate::model::Model;

thread_local! {
	static MODEL: ArcRwSignal<Model> = ArcRwSignal::new(Model::default());
}

pub fn read<T>(f: impl FnOnce(&Model) -> T) -> T {
	MODEL.with(|model| model.with(f))
}

pub fn update(f: impl FnOnce(&mut Model)) {
	MODEL.with(|model| model.update(f));
}

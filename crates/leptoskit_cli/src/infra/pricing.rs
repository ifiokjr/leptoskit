//! Fetch provider prices at the time of the decision; never silently pick a
//! more expensive size or exchange currencies.
use anyhow::Context;
use anyhow::Result;
use anyhow::ensure;
use leptoskit_config::infra::ComputeProvider;
use leptoskit_config::infra::InfraConfig;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;

#[derive(Debug, Serialize, Deserialize)]
struct Offer {
	provider: String,
	size: String,
	location: String,
	memory_gb: f64,
	currency: String,
	monthly_compute_ipv4: f64,
}

fn get(url: &str, token: &str) -> Result<Value> {
	let agent: ureq::Agent = ureq::Agent::config_builder()
		.timeout_global(Some(std::time::Duration::from_secs(30)))
		.build()
		.into();
	let mut response = agent
		.get(url)
		.header("Authorization", format!("Bearer {token}"))
		.call()
		.map_err(|_| {
			anyhow::anyhow!("provider price request failed; check credentials and network")
		})?;
	serde_json::from_str(&response.body_mut().read_to_string()?)
		.context("invalid provider price response")
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>> {
	value[key]
		.as_array()
		.with_context(|| format!("provider response lacks {key}"))
}

fn number(value: &Value) -> Result<f64> {
	let amount = value
		.as_f64()
		.or_else(|| value.as_str().and_then(|s| s.parse().ok()))
		.context("missing provider price")?;
	ensure!(
		amount.is_finite() && amount >= 0.0,
		"invalid provider price"
	);
	Ok(amount)
}

fn hetzner(types: &[Value], pricing: &Value) -> Result<Vec<Offer>> {
	let currency = pricing["currency"]
		.as_str()
		.context("missing provider currency")?;
	let ipv4 = array(pricing, "primary_ips")?
		.iter()
		.find(|v| v["type"] == "ipv4")
		.context("missing primary IPv4 prices")?;
	let mut offers = Vec::new();
	for size in types {
		if size["architecture"] != "x86" || size["deprecated"] == true {
			continue;
		}
		for price in array(size, "prices")? {
			let location = price["location"].as_str().context("missing location")?;
			if let Some(locations) = size["locations"].as_array()
				&& !locations.iter().any(|v| {
					v["name"] == location && v["available"] == true && v["deprecation"].is_null()
				}) {
				continue;
			}
			let ip = array(ipv4, "prices")?
				.iter()
				.find(|p| p["location"] == location)
				.context("missing regional IPv4 price")?;
			offers.push(Offer {
				provider: "hetzner".into(),
				size: size["name"].as_str().context("missing size")?.into(),
				location: location.into(),
				memory_gb: number(&size["memory"])?,
				currency: currency.into(),
				monthly_compute_ipv4: number(&price["price_monthly"]["net"])?
					+ number(&ip["price_monthly"]["net"])?,
			});
		}
	}
	Ok(offers)
}

fn digitalocean(sizes: &[Value]) -> Result<Vec<Offer>> {
	let mut offers = Vec::new();
	for size in sizes {
		if size["available"] != true {
			continue;
		}
		for region in array(size, "regions")? {
			offers.push(Offer {
				provider: "digitalocean".into(),
				size: size["slug"].as_str().context("missing size")?.into(),
				location: region.as_str().context("missing region")?.into(),
				memory_gb: number(&size["memory"])? / 1024.0,
				currency: "USD".into(),
				monthly_compute_ipv4: number(&size["price_monthly"])?,
			});
		}
	}
	Ok(offers)
}

fn fetch(provider: ComputeProvider) -> Result<Vec<Offer>> {
	let (key, base, list) = match provider {
		ComputeProvider::Hetzner => {
			(
				"HCLOUD_TOKEN",
				"https://api.hetzner.cloud/v1",
				"server_types",
			)
		}
		ComputeProvider::Digitalocean => {
			(
				"DIGITALOCEAN_TOKEN",
				"https://api.digitalocean.com/v2",
				"sizes",
			)
		}
	};
	let token =
		std::env::var(key).with_context(|| format!("set {key} to read current provider offers"))?;
	let mut items = Vec::new();
	for page in 1..=100 {
		let response = get(&format!("{base}/{list}?per_page=50&page={page}"), &token)?;
		let values = array(&response, list)?;
		items.extend(values.iter().cloned());
		if values.len() < 50 {
			return match provider {
				ComputeProvider::Hetzner => {
					hetzner(&items, &get(&format!("{base}/pricing"), &token)?["pricing"])
				}
				ComputeProvider::Digitalocean => digitalocean(&items),
			};
		}
	}
	anyhow::bail!("provider catalog exceeded 100 pages")
}

pub(super) fn print(minimum: f64) -> Result<()> {
	ensure!(
		minimum.is_finite() && minimum > 0.0,
		"minimum memory must be positive"
	);
	let mut offers = Vec::new();
	for (provider, key) in [
		(ComputeProvider::Hetzner, "HCLOUD_TOKEN"),
		(ComputeProvider::Digitalocean, "DIGITALOCEAN_TOKEN"),
	] {
		if std::env::var_os(key).is_some() {
			offers.extend(fetch(provider)?);
		}
	}
	ensure!(
		!offers.is_empty(),
		"set HCLOUD_TOKEN and/or DIGITALOCEAN_TOKEN to compare current offers"
	);
	offers.retain(|v| v.memory_gb >= minimum);
	offers.sort_by(|a, b| {
		a.currency
			.cmp(&b.currency)
			.then(a.monthly_compute_ipv4.total_cmp(&b.monthly_compute_ipv4))
			.then(a.size.cmp(&b.size))
			.then(a.location.cmp(&b.location))
	});
	println!(
		"{}",
		serde_json::to_string_pretty(
			&json!({"fetched_at": chrono::Utc::now().to_rfc3339(), "offers": offers,
		"excludes": ["tax", "storage", "egress over allowance", "domains", "other services"], "currency_conversion": false})
		)?
	);
	Ok(())
}

pub(super) fn guard(c: &InfraConfig) -> Result<()> {
	let Some(budget) = &c.budget else {
		return Ok(());
	};
	let offers = fetch(c.compute)?;
	let selected = offers
		.iter()
		.find(|o| o.size == c.server_type && o.location == c.location)
		.context("selected size/location is unavailable in current provider catalog")?;
	ensure!(
		selected.currency == budget.currency,
		"budget currency differs from provider; no automatic currency conversion"
	);
	let total = selected.monthly_compute_ipv4 * f64::from(c.instances);
	ensure!(
		total <= budget.monthly_compute,
		"base monthly compute + IPv4 estimate {total:.2} {} exceeds configured budget {:.2}; no \
		 infrastructure mutation was attempted",
		selected.currency,
		budget.monthly_compute
	);
	println!(
		"Base compute + IPv4 estimate: {total:.2} {}/month; excludes tax, storage, excess \
		 traffic, and other services",
		selected.currency
	);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn catalog_prices_include_ipv4_and_exclude_unavailable_and_arm_sizes() {
		let prices = json!({"currency":"EUR", "primary_ips":[{"type":"ipv4", "prices":[{"location":"hel1", "price_monthly":{"net":"0.50"}}]}]});
		let size = json!({"name":"cx23", "architecture":"x86", "memory":4, "locations":[{"name":"hel1", "available":true}], "prices":[{"location":"hel1", "price_monthly":{"net":"3.49"}}]});
		let offers = hetzner(std::slice::from_ref(&size), &prices).unwrap();
		assert!((offers[0].monthly_compute_ipv4 - 3.99).abs() < 0.0001);
		assert_eq!(offers[0].currency, "EUR");
		let mut unavailable = size.clone();
		unavailable["locations"][0]["available"] = json!(false);
		let mut arm = size;
		arm["architecture"] = json!("arm");
		assert!(hetzner(&[unavailable, arm], &prices).unwrap().is_empty());
		let offers = digitalocean(&[json!({"slug":"s-1vcpu-2gb", "memory":2048, "available":true, "regions":["ams3"], "price_monthly":12})]).unwrap();
		assert_eq!(offers[0].memory_gb, 2.0);
		assert_eq!(offers[0].currency, "USD");
	}

	fn prices() -> Value {
		json!({"currency":"EUR", "primary_ips":[{"type":"ipv4", "prices":[{"location":"hel1", "price_monthly":{"net":"0.50"}}]}]})
	}

	fn size() -> Value {
		json!({"name":"cx23", "architecture":"x86", "memory":4, "locations":[{"name":"hel1", "available":true}], "prices":[{"location":"hel1", "price_monthly":{"net":"3.49"}}]})
	}

	#[test]
	fn provider_arrays_and_amounts_are_validated_instead_of_defaulting() {
		assert!(
			array(&json!({}), "server_types")
				.unwrap_err()
				.to_string()
				.contains("provider response lacks server_types")
		);
		assert!(array(&json!({"server_types": 7}), "server_types").is_err());
		assert_eq!(number(&json!(4)).unwrap(), 4.0);
		assert_eq!(number(&json!("3.49")).unwrap(), 3.49);
		assert!(
			number(&json!("not-a-price"))
				.unwrap_err()
				.to_string()
				.contains("missing provider price")
		);
		assert!(
			number(&json!(-1.0))
				.unwrap_err()
				.to_string()
				.contains("invalid provider price")
		);
		assert!(number(&json!(null)).is_err());
	}

	#[test]
	fn hetzner_rejects_incomplete_catalogs_and_skips_undeprecatable_rows() {
		assert!(
			hetzner(&[], &json!({}))
				.unwrap_err()
				.to_string()
				.contains("missing provider currency")
		);
		assert!(hetzner(&[], &json!({"currency": "EUR"})).is_err());
		assert!(
			hetzner(&[], &json!({"currency": "EUR", "primary_ips": []}))
				.unwrap_err()
				.to_string()
				.contains("missing primary IPv4 prices")
		);
		let deprecated = json!({
			"name": "cx22", "architecture": "x86", "memory": 4, "deprecated": true,
			"prices": [{"location": "hel1", "price_monthly": {"net": "3.49"}}],
		});
		assert!(hetzner(&[deprecated], &prices()).unwrap().is_empty());

		let mut missing_location = size();
		missing_location["prices"][0]
			.as_object_mut()
			.unwrap()
			.remove("location");
		assert!(
			hetzner(&[missing_location], &prices())
				.unwrap_err()
				.to_string()
				.contains("missing location")
		);
		let mut unlisted = size();
		unlisted.as_object_mut().unwrap().remove("locations");
		unlisted["prices"][0]["location"] = json!("fsn1");
		assert!(
			hetzner(&[unlisted], &prices())
				.unwrap_err()
				.to_string()
				.contains("missing regional IPv4 price")
		);
		let mut unnamed = size();
		unnamed.as_object_mut().unwrap().remove("name");
		assert!(
			hetzner(&[unnamed], &prices())
				.unwrap_err()
				.to_string()
				.contains("missing size")
		);
		let mut unpriced = size();
		unpriced["prices"][0]["price_monthly"] = json!({});
		assert!(hetzner(&[unpriced], &prices()).is_err());
	}

	#[test]
	fn digitalocean_skips_unavailable_sizes_and_validates_each_row() {
		let unavailable = json!({
			"slug": "s-1vcpu-1gb", "memory": 1024, "available": false,
			"regions": ["ams3"], "price_monthly": 6,
		});
		assert!(digitalocean(&[unavailable]).unwrap().is_empty());

		let no_regions = json!({
			"slug": "s-1vcpu-1gb", "memory": 1024, "available": true, "price_monthly": 6,
		});
		assert!(
			digitalocean(&[no_regions])
				.unwrap_err()
				.to_string()
				.contains("lacks regions")
		);
		let no_slug = json!({
			"memory": 1024, "available": true, "regions": ["ams3"], "price_monthly": 6,
		});
		assert!(
			digitalocean(&[no_slug])
				.unwrap_err()
				.to_string()
				.contains("missing size")
		);
		let bad_region = json!({
			"slug": "s-1vcpu-1gb", "memory": 1024, "available": true,
			"regions": [7], "price_monthly": 6,
		});
		assert!(
			digitalocean(&[bad_region])
				.unwrap_err()
				.to_string()
				.contains("missing region")
		);
		let no_price = json!({
			"slug": "s-1vcpu-1gb", "memory": 1024, "available": true, "regions": ["ams3"],
		});
		assert!(digitalocean(&[no_price]).is_err());
	}

	#[test]
	fn offer_queries_require_a_positive_minimum_memory() {
		for minimum in [0.0, -1.0, f64::NAN, f64::INFINITY] {
			assert!(
				print(minimum)
					.unwrap_err()
					.to_string()
					.contains("minimum memory must be positive")
			);
		}
	}

	#[test]
	fn a_missing_budget_never_contacts_a_provider() {
		let infra = config(None);
		guard(&infra).unwrap();
	}

	fn config(budget: Option<&str>) -> InfraConfig {
		let budget = budget.unwrap_or_default();
		let raw = format!(
			"[app]\nname = 'notes'\n[db]\ndialect = 'postgres'\n[infra]\nname = \
			 'notes-production'\ndomain = 'app.example.com'\nlocation = 'hel1'\nserver_type = \
			 'cx23'\nssh_key_ids = [123]\nssh_cidrs = ['192.0.2.1/32']\naccount_id = \
			 '{}'\nzone_id = '{}'\nimage = 'ghcr.io/team/notes'\nregistry = \
			 'ghcr.io'\nregistry_username = 'team'\nupdates_domain = \
			 'updates.example.com'\nupdates_bucket = 'notes-releases'\nbackup_bucket = \
			 'notes-backups'\n{budget}",
			"a".repeat(32),
			"b".repeat(32)
		);
		leptoskit_config::LeptoskitConfig::from_toml(&raw, std::path::Path::new("."))
			.unwrap()
			.infra
			.unwrap()
	}
}

use leptoskit_config::infra::InfraConfig;
use serde_json::Value;
use serde_json::json;

const EXTERNAL_CLOUD_INIT: &str =
	"#cloud-config\nssh_pwauth: false\ndisable_root: false\npackage_update: true\npackages:\n  - \
	 ca-certificates\n  - libstdc++6\n  - unattended-upgrades\n  - restic\n  - \
	 python3\nwrite_files:\n  - path: /etc/ssh/sshd_config.d/00-leptoskit.conf\n    content: |\n      \
	 PasswordAuthentication no\n      PermitRootLogin prohibit-password\nruncmd:\n  - sshd -t\n  \
	 - systemctl restart ssh\n";

fn external(value: &mut Value, c: &InfraConfig) {
	let Some(external) = &c.external else { return };
	for kind in ["hcloud_server", "digitalocean_droplet"] {
		if let Some(servers) = value["resource"]
			.get_mut(kind)
			.and_then(Value::as_object_mut)
		{
			for server in servers.values_mut() {
				server["user_data"] = json!(EXTERNAL_CLOUD_INIT);
			}
		}
	}
	if let Some(rules) = value["resource"]
		.get_mut("digitalocean_firewall")
		.and_then(|firewall| firewall.get_mut("app"))
		.and_then(|app| app.get_mut("inbound_rule"))
		.and_then(Value::as_array_mut)
	{
		rules.retain(|rule| rule["port_range"] == "22");
	}
	value["resource"]["cloudflare_zero_trust_tunnel_cloudflared_config"]["app"]["config"]
		["ingress"][0]["service"] = json!(format!("http://127.0.0.1:{}", external.origin_port));
	let resources = value["resource"].as_object_mut().unwrap();
	resources.remove("cloudflare_r2_custom_domain");
	let buckets = resources
		.get_mut("cloudflare_r2_bucket")
		.unwrap()
		.as_object_mut()
		.unwrap();
	buckets.remove("updates");
	if c.backup_bucket.is_empty() {
		buckets.remove("backups");
	}
	if buckets.is_empty() {
		resources.remove("cloudflare_r2_bucket");
	}
	let outputs = value["output"].as_object_mut().unwrap();
	for name in ["updates_url", "updates_bucket"] {
		outputs.remove(name);
	}
	if c.backup_bucket.is_empty() {
		for name in ["backup_bucket", "backup_endpoint", "s3_endpoint"] {
			outputs.remove(name);
		}
	}
}

fn extend(value: &mut Value, c: &InfraConfig) {
	use leptoskit_config::infra::ComputeProvider;
	let replicas: serde_json::Map<String, Value> = (1..c.instances)
		.map(|i| (i.to_string(), json!(format!("10.42.0.{}", 10 + i))))
		.collect();
	let server = value["resource"]["hcloud_server"]["app"]
		.as_object_mut()
		.expect("generated server");
	server["user_data"] = json!(server["user_data"].as_str().expect("cloud init").replace(
		"  - unattended-upgrades\n",
		"  - unattended-upgrades\n  - restic\n  - python3\n"
	));
	let cloud_init = server["user_data"].clone();
	if c.compute == ComputeProvider::Hetzner {
		let zone = match c.location.as_str() {
			"ash" => "us-east",
			"hil" => "us-west",
			"sin" => "ap-southeast",
			_ => "eu-central",
		};
		value["resource"]["hcloud_network"] = json!({"app": {"name": format!("{}-private", c.name), "ip_range": "10.42.0.0/16", "lifecycle": {"prevent_destroy": true}}});
		value["resource"]["hcloud_network_subnet"] = json!({"app": {"network_id": "${hcloud_network.app.id}", "type": "cloud", "network_zone": zone, "ip_range": "10.42.0.0/24"}});
		value["resource"]["hcloud_server"]["app"]["network"] =
			json!([{"network_id": "${hcloud_network.app.id}", "ip": "10.42.0.10"}]);
		value["resource"]["hcloud_server"]["app"]["depends_on"] =
			json!(["hcloud_network_subnet.app"]);
		let mut replica = value["resource"]["hcloud_server"]["app"].clone();
		replica["for_each"] = json!(replicas);
		replica["name"] = json!(format!("{}-${{each.key}}", c.name));
		replica["network"][0]["ip"] = json!("${each.value}");
		replica
			.as_object_mut()
			.expect("replica")
			.remove("lifecycle");
		value["resource"]["hcloud_server"]["replica"] = replica;
		value["output"]["server_ips"] = json!({"value": "${concat([hcloud_server.app.ipv4_address], [for s in values(hcloud_server.replica) : s.ipv4_address])}"});
		value["output"]["private_ip"] = json!({"value": "${one(hcloud_server.app.network).ip}"});
	} else {
		value["terraform"]["required_providers"]
			.as_object_mut()
			.unwrap()
			.remove("hcloud");
		value["provider"].as_object_mut().unwrap().remove("hcloud");
		value["resource"]
			.as_object_mut()
			.unwrap()
			.remove("hcloud_server");
		value["resource"]
			.as_object_mut()
			.unwrap()
			.remove("hcloud_firewall");
		value["terraform"]["required_providers"]["digitalocean"] =
			json!({"source": "digitalocean/digitalocean", "version": "~> 2.0"});
		value["provider"]["digitalocean"] = json!({});
		let primary = json!({"name": c.name, "image": "ubuntu-24-04-x64", "region": c.location, "size": c.server_type,
			"ssh_keys": c.ssh_key_ids.iter().map(ToString::to_string).collect::<Vec<_>>(), "ipv6": true, "monitoring": true,
			"user_data": cloud_init, "lifecycle": {"prevent_destroy": true}});
		let mut replica = primary.clone();
		replica["for_each"] = json!(replicas);
		replica["name"] = json!(format!("{}-${{each.key}}", c.name));
		replica.as_object_mut().unwrap().remove("lifecycle");
		value["resource"]["digitalocean_droplet"] = json!({"app": primary, "replica": replica});
		let hosts = "${concat([digitalocean_droplet.app.id], [for s in \
		             values(digitalocean_droplet.replica) : s.id])}";
		value["resource"]["digitalocean_firewall"] = json!({"app": {"name": format!("{}-origin", c.name), "droplet_ids": hosts,
			"inbound_rule": [{"protocol": "tcp", "port_range": "22", "source_addresses": c.ssh_cidrs},
				{"protocol": "tcp", "port_range": "5432", "source_droplet_ids": hosts},
				{"protocol": "tcp", "port_range": "4222", "source_droplet_ids": hosts},
				{"protocol": "tcp", "port_range": "6379", "source_droplet_ids": hosts}],
			"outbound_rule": [{"protocol": "tcp", "port_range": "1-65535", "destination_addresses": ["0.0.0.0/0", "::/0"]},
				{"protocol": "udp", "port_range": "1-65535", "destination_addresses": ["0.0.0.0/0", "::/0"]},
				{"protocol": "icmp", "destination_addresses": ["0.0.0.0/0", "::/0"]}]}});
		value["output"]["server_ip"] = json!({"value": "${digitalocean_droplet.app.ipv4_address}"});
		value["output"]["server_ips"] = json!({"value": "${concat([digitalocean_droplet.app.ipv4_address], [for s in values(digitalocean_droplet.replica) : s.ipv4_address])}"});
		value["output"]["private_ip"] =
			json!({"value": "${digitalocean_droplet.app.ipv4_address_private}"});
	}
	if let Some(b2) = &c.b2 {
		value["resource"]["cloudflare_r2_bucket"]
			.as_object_mut()
			.unwrap()
			.remove("backups");
		value["terraform"]["required_providers"]["b2"] =
			json!({"source": "Backblaze/b2", "version": ">= 0.10, < 1.0"});
		value["provider"]["b2"] = json!({});
		value["resource"]["b2_bucket"] = json!({"backups": {"bucket_name": c.backup_bucket, "bucket_type": "allPrivate", "lifecycle": {"prevent_destroy": true}}});
		// B2 keeps hidden versions after S3 deletion. Reclaim only versions
		// already superseded/deleted by Restic, never live repository objects.
		value["resource"]["b2_bucket"]["backups"]["lifecycle_rules"] = json!([{
			"file_name_prefix": "", "days_from_hiding_to_deleting": 1,
			"days_from_starting_to_canceling_unfinished_large_files": 7
		}]);
		value["output"]["backup_bucket"] = json!({"value": "${b2_bucket.backups.bucket_name}"});
		value["output"]["backup_endpoint"] =
			json!({"value": format!("https://s3.{}.backblazeb2.com", b2.region)});
	} else {
		value["output"]["backup_endpoint"] = value["output"]["s3_endpoint"].clone();
	}
}

pub(super) fn resources(c: &InfraConfig) -> Value {
	let mut value = json!({
		"terraform": {"required_version": ">= 1.10, < 2.0", "required_providers": {
			"hcloud": {"source": "hetznercloud/hcloud", "version": "~> 1.0"},
			"cloudflare": {"source": "cloudflare/cloudflare", "version": "~> 5.0"}}},
		"provider": {"hcloud": {}, "cloudflare": {}},
		"resource": {
			"hcloud_firewall": {"app": {"name": format!("{}-origin", c.name), "rule": [
				{"direction": "in", "protocol": "tcp", "port": "22", "source_ips": c.ssh_cidrs},
				{"direction": "in", "protocol": "icmp", "source_ips": ["0.0.0.0/0", "::/0"]}]}},
			"hcloud_server": {"app": {"name": c.name, "image": "ubuntu-24.04", "location": c.location,
				"server_type": c.server_type, "ssh_keys": c.ssh_key_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
				"firewall_ids": ["${hcloud_firewall.app.id}"], "public_net": [{"ipv4_enabled": true, "ipv6_enabled": true}],
				"lifecycle": {"prevent_destroy": true}, "user_data": "#cloud-config\nssh_pwauth: false\ndisable_root: false\npackage_update: true\npackages:\n  - docker.io\n  - unattended-upgrades\nwrite_files:\n  - path: /etc/ssh/sshd_config.d/00-leptoskit.conf\n    content: |\n      PasswordAuthentication no\n      PermitRootLogin prohibit-password\n  - path: /etc/docker/daemon.json\n    content: '{\"log-driver\":\"local\",\"log-opts\":{\"max-size\":\"10m\",\"max-file\":\"3\"}}'\nruncmd:\n  - systemctl enable --now docker\n  - sshd -t\n  - systemctl restart ssh\n"}},
			"cloudflare_zero_trust_tunnel_cloudflared": {"app": {"account_id": c.account_id, "name": c.name, "config_src": "cloudflare"}},
			"cloudflare_zero_trust_tunnel_cloudflared_config": {"app": {"account_id": c.account_id,
				"tunnel_id": "${cloudflare_zero_trust_tunnel_cloudflared.app.id}", "config": {"ingress": [
					{"hostname": c.domain, "service": "http://kamal-proxy:80"}, {"service": "http_status:404"}]}}},
			"cloudflare_dns_record": {"app": {"zone_id": c.zone_id, "name": c.domain, "type": "CNAME", "ttl": 1,
				"proxied": true, "content": "${cloudflare_zero_trust_tunnel_cloudflared.app.id}.cfargotunnel.com"}},
			"cloudflare_r2_bucket": {
				"updates": {"account_id": c.account_id, "name": c.updates_bucket, "lifecycle": {"prevent_destroy": true}},
				"backups": {"account_id": c.account_id, "name": c.backup_bucket, "lifecycle": {"prevent_destroy": true}}},
			"cloudflare_r2_custom_domain": {"updates": {"account_id": c.account_id,
				"bucket_name": "${cloudflare_r2_bucket.updates.name}", "domain": c.updates_domain,
				"zone_id": c.zone_id, "enabled": true, "min_tls": "1.2"}}
		},
		"data": {"cloudflare_zero_trust_tunnel_cloudflared_token": {"app": {"account_id": c.account_id,
			"tunnel_id": "${cloudflare_zero_trust_tunnel_cloudflared.app.id}"}}},
		"output": {"server_ip": {"value": "${hcloud_server.app.ipv4_address}"},
			"app_url": {"value": format!("https://{}", c.domain)},
			"updates_url": {"value": format!("https://{}", c.updates_domain)},
			"updates_bucket": {"value": "${cloudflare_r2_bucket.updates.name}"},
			"backup_bucket": {"value": "${cloudflare_r2_bucket.backups.name}"},
			"s3_endpoint": {"value": format!("https://{}.r2.cloudflarestorage.com", c.account_id)},
			"tunnel_token": {"value": "${data.cloudflare_zero_trust_tunnel_cloudflared_token.app.token}", "sensitive": true}}
	});
	if let Some(state) = &c.state {
		value["terraform"]["backend"] = json!({"s3": {"bucket": state.bucket, "key": state.key,
			"region": state.region, "encrypt": true, "use_lockfile": true}});
	}
	if c.hosted_state {
		value["terraform"]["backend"] = json!({"http":{}});
	}
	extend(&mut value, c);
	external(&mut value, c);
	value
}

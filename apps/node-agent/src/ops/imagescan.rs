//! Image checks (T11.12): known vulnerabilities (Trivy) and whether a registry has moved on.
//!
//! Trivy runs once per image in a container of its own: it reads the image straight from the
//! registry (so no Docker socket is mounted anywhere), keeps its database in a folder of the
//! agent's state, and writes its report to a file the agent then reads. Only images of the
//! panel's own allow-list are scanned.

use std::{sync::Arc, time::Duration};

use builtpanel_docker_manager::{ImageRef, Network, Tmpfs, VolumeMount};
use builtpanel_node_protocol::{
    AgentError, ErrorCode, ImageDigest, ImageVulnerability, Operation, Output,
};
use builtpanel_tenant_stack::{Allowlist, Role};

use super::{
    db,
    scan::{base_spec, state},
    sqlimport::failed,
    tenant::{TenantContext, docker_error},
};

const NETWORK: &str = "bp-trivy-net";
const LIMIT: Duration = Duration::from_secs(30 * 60);
const MAX_TOP: usize = 50;

fn clip(text: &str, max: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(max).collect()
}

/// Reads Trivy's JSON report: counts by severity and the worst findings.
///
/// # Errors
/// The text is not a Trivy report.
pub fn parse_report(text: &str) -> Result<(u32, u32, u32, u32, Vec<ImageVulnerability>), String> {
    let report: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("unreadable report: {e}"))?;
    let results = report
        .get("Results")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let (mut critical, mut high, mut medium, mut low) = (0u32, 0u32, 0u32, 0u32);
    let mut all = Vec::new();
    for result in &results {
        let Some(list) = result
            .get("Vulnerabilities")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for v in list {
            let text = |key: &str| v.get(key).and_then(serde_json::Value::as_str).unwrap_or("");
            let severity = text("Severity").to_ascii_uppercase();
            match severity.as_str() {
                "CRITICAL" => critical += 1,
                "HIGH" => high += 1,
                "MEDIUM" => medium += 1,
                "LOW" => low += 1,
                _ => continue,
            }
            all.push(ImageVulnerability {
                id: clip(text("VulnerabilityID"), 64),
                package: clip(text("PkgName"), 100),
                installed: clip(text("InstalledVersion"), 64),
                fixed: clip(text("FixedVersion"), 64),
                severity,
                title: clip(text("Title"), 200),
            });
        }
    }
    let rank = |s: &str| match s {
        "CRITICAL" => 0,
        "HIGH" => 1,
        "MEDIUM" => 2,
        _ => 3,
    };
    // Worst first; among equals the ones a newer version fixes (there is something to do).
    all.sort_by(|a, b| {
        rank(&a.severity)
            .cmp(&rank(&b.severity))
            .then(a.fixed.is_empty().cmp(&b.fixed.is_empty()))
            .then_with(|| a.id.cmp(&b.id))
    });
    all.truncate(MAX_TOP);
    Ok((critical, high, medium, low, all))
}

/// `scan_image`.
///
/// # Errors
/// [`AgentError`]: an image that is not on the allow-list (not retryable), Docker or the
/// registry failing (retryable), Trivy ending badly.
pub async fn scan_image(context: Arc<TenantContext>, op: Operation) -> Result<Output, AgentError> {
    use std::os::unix::fs::PermissionsExt as _;
    let Operation::ScanImage { reference } = op else {
        return Err(super::wrong_operation());
    };
    let allowlist = Allowlist::embedded().map_err(|e| failed(&format!("the image list: {e}")))?;
    let target = ImageRef::parse(&reference)
        .ok()
        .filter(|r| allowlist.entries().iter().any(|e| e.reference == *r))
        .ok_or_else(|| {
            AgentError::new(
                ErrorCode::InvalidArguments,
                "that image is not on the allow-list",
            )
        })?;
    let trivy = allowlist
        .current(Role::Trivy)
        .map_err(|e| failed(&e.to_string()))?
        .clone();
    context
        .docker
        .pull_image(&trivy)
        .await
        .map_err(|e| docker_error(&e))?;
    let base = state(&context)?.join("trivy");
    let (cache, out) = (base.join("cache"), base.join("out"));
    for dir in [&cache, &out] {
        if !dir.is_dir() {
            std::fs::create_dir_all(dir)
                .map_err(|e| failed(&format!("cannot make the folder: {e}")))?;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o777))
                .map_err(|e| failed(&format!("cannot set the folder's rights: {e}")))?;
        }
    }
    let state_dir = state(&context)?;
    let cache_volume = db::name("bp-trivy-cache")?;
    let out_volume = db::name("bp-trivy-out")?;
    for (volume, dir) in [(&cache_volume, &cache), (&out_volume, &out)] {
        context
            .docker
            .create_bind_volume(volume, dir, &state_dir)
            .await
            .map_err(|e| docker_error(&e))?;
    }
    let net = db::name(NETWORK)?;
    context
        .docker
        .ensure_network(&net, false)
        .await
        .map_err(|e| docker_error(&e))?;
    let file = format!("scan-{:016x}.json", {
        use std::hash::{Hash as _, Hasher as _};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        reference.hash(&mut h);
        h.finish()
    });
    let mut spec = base_spec(
        db::name("bp-trivy-scan")?,
        trivy,
        &[
            "trivy",
            "image",
            "--quiet",
            "--scanners",
            "vuln",
            "--image-src",
            "remote",
            "--cache-dir",
            "/cache",
            "--format",
            "json",
            "--output",
            &format!("/out/{file}"),
            target.as_str(),
        ],
    );
    spec.cap_add = Vec::new();
    spec.writable_root = true;
    spec.network = Network::Named(net);
    spec.tmpfs = vec![Tmpfs {
        target: "/tmp".into(),
        size_bytes: 512 * 1024 * 1024,
    }];
    spec.volumes = vec![
        VolumeMount {
            volume: cache_volume,
            target: "/cache".into(),
            read_only: false,
        },
        VolumeMount {
            volume: out_volume,
            target: "/out".into(),
            read_only: false,
        },
    ];
    let _ = context.docker.remove(&spec.name, true).await;
    let done = context
        .docker
        .run_to_end(&spec, LIMIT)
        .await
        .map_err(|e| docker_error(&e))?;
    let path = out.join(&file);
    let text = std::fs::read_to_string(&path);
    let _ = std::fs::remove_file(&path);
    if done.exit_code != 0 {
        let tail: String = done
            .output
            .lines()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(AgentError::new(
            ErrorCode::Internal,
            format!("trivy ended with {}: {}", done.exit_code, clip(&tail, 300)),
        )
        .retryable());
    }
    let text = text.map_err(|e| failed(&format!("no report: {e}")))?;
    let (critical, high, medium, low, top) = parse_report(&text).map_err(|e| failed(&e))?;
    Ok(Output::ImageScanned {
        critical,
        high,
        medium,
        low,
        top,
    })
}

/// `check_image_updates`.
///
/// # Errors
/// [`AgentError`]: more than 50 images (not retryable). An image a registry cannot be asked
/// about is left out of the answer.
pub async fn check_image_updates(
    context: Arc<TenantContext>,
    op: Operation,
) -> Result<Output, AgentError> {
    let Operation::CheckImageUpdates { images } = op else {
        return Err(super::wrong_operation());
    };
    if images.len() > 50 {
        return Err(AgentError::new(
            ErrorCode::InvalidArguments,
            "at most 50 images at a time",
        ));
    }
    let mut items = Vec::new();
    for image in images {
        if let Ok(digest) = context.docker.registry_digest(&image).await
            && digest.starts_with("sha256:")
        {
            items.push(ImageDigest { image, digest });
        }
    }
    Ok(Output::ImageDigests { items })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_trivy_report_is_counted_and_the_worst_come_first() {
        let report = r#"{"Results":[{"Target":"x","Vulnerabilities":[
            {"VulnerabilityID":"CVE-1","PkgName":"a","InstalledVersion":"1","FixedVersion":"","Severity":"HIGH","Title":"t1"},
            {"VulnerabilityID":"CVE-2","PkgName":"b","InstalledVersion":"1","FixedVersion":"2","Severity":"HIGH","Title":"t2"},
            {"VulnerabilityID":"CVE-3","PkgName":"c","InstalledVersion":"1","FixedVersion":"2","Severity":"CRITICAL","Title":"t3"},
            {"VulnerabilityID":"CVE-4","PkgName":"d","InstalledVersion":"1","Severity":"LOW"},
            {"VulnerabilityID":"CVE-5","PkgName":"e","InstalledVersion":"1","Severity":"UNKNOWN"}]},
            {"Target":"empty"}]}"#;
        let (critical, high, medium, low, top) = parse_report(report).unwrap();
        assert_eq!((critical, high, medium, low), (1, 2, 0, 1));
        assert_eq!(
            top.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
            ["CVE-3", "CVE-2", "CVE-1", "CVE-4"]
        );
        assert!(parse_report("not json").is_err());
        assert_eq!(parse_report("{}").unwrap().0, 0);
    }
}

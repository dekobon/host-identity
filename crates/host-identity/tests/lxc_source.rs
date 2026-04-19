//! End-to-end tests for [`LxcId`] against a synthesised procfs.

#![cfg(all(target_os = "linux", feature = "container"))]

use std::io::Write;

use host_identity::sources::{ContainerId, LxcId};
use host_identity::{Resolver, SourceKind, Wrap};
use tempfile::NamedTempFile;

struct FakeProcfs {
    cgroup: NamedTempFile,
    mountinfo: NamedTempFile,
    machine_id: NamedTempFile,
}

impl FakeProcfs {
    fn new(cgroup: &str, mountinfo: &str, machine_id: &str) -> Self {
        let mut cg = NamedTempFile::new().unwrap();
        cg.write_all(cgroup.as_bytes()).unwrap();
        let mut mi = NamedTempFile::new().unwrap();
        mi.write_all(mountinfo.as_bytes()).unwrap();
        let mut id = NamedTempFile::new().unwrap();
        id.write_all(machine_id.as_bytes()).unwrap();
        Self {
            cgroup: cg,
            mountinfo: mi,
            machine_id: id,
        }
    }

    fn lxc(&self) -> LxcId {
        LxcId::new()
            .with_cgroup(self.cgroup.path())
            .with_mountinfo(self.mountinfo.path())
            .with_machine_id(self.machine_id.path())
    }

    fn container(&self) -> ContainerId {
        ContainerId::at(self.mountinfo.path())
    }
}

#[test]
fn resolves_lxc_source_with_stable_uuid() {
    let fake = FakeProcfs::new("0::/lxc.payload.demo\n", "", "host-machine-id\n");

    let a = Resolver::new().push(fake.lxc()).resolve().unwrap();
    let b = Resolver::new().push(fake.lxc()).resolve().unwrap();

    assert_eq!(a.source(), SourceKind::Lxc);
    assert_eq!(a.as_uuid(), b.as_uuid());
    assert_eq!(
        a.as_uuid(),
        Wrap::UuidV5Namespaced
            .apply("lxc:host-machine-id:demo")
            .unwrap()
    );
}

#[test]
fn container_id_wins_over_lxc_when_both_match() {
    // Docker-in-LXC: mountinfo carries both an OCI 64-hex runtime
    // token *and* an lxc.payload bind-mount source. The chain puts
    // ContainerId first; it must short-circuit before LxcId runs.
    let hex = "a".repeat(64);
    let mountinfo = format!(
        "1 2 0:0 / /host rw - overlay overlay rw,lowerdir=/var/lib/docker/containers/{hex}/hostname\n\
         3 4 0:0 / /root rw - overlay overlay rw,lowerdir=/lxc.payload.outer/rootfs\n"
    );
    let fake = FakeProcfs::new("0::/lxc.payload.outer\n", &mountinfo, "host-machine-id\n");

    let id = Resolver::new()
        .push(fake.container())
        .push(fake.lxc())
        .resolve()
        .unwrap();

    assert_eq!(id.source(), SourceKind::Container);
    assert_eq!(id.as_uuid(), Wrap::UuidV5Namespaced.apply(&hex).unwrap());
}

#[test]
fn lxc_falls_through_when_no_markers_present() {
    // No LXC markers anywhere → LxcId returns Ok(None), resolver walks
    // on to the next source.
    let fake = FakeProcfs::new(
        "0::/user.slice/user-1000.slice/user@1000.service\n",
        "1 2 0:0 / /host rw - overlay overlay rw,lowerdir=/var/lib/foo/bar\n",
        "host-machine-id\n",
    );
    let mut fallback = NamedTempFile::new().unwrap();
    writeln!(fallback, "fallback-id").unwrap();

    let id = Resolver::new()
        .push(fake.lxc())
        .push(host_identity::sources::FileOverride::new(fallback.path()))
        .resolve()
        .unwrap();

    assert_eq!(id.source(), SourceKind::FileOverride);
}

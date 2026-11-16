//! AMD fork (J6): the files amdgpu loads for an RDNA 4 Radeon, pinned as
//! the NVIDIA GSP-RM files are ([`super::GSP`]).
//!
//! The Navi 48 (RX 9070 series, PCI 0x7550) runs GC 12.0.1, PSP 14.0.3,
//! SMU 14.0.3, SDMA 7.0.1, DCN 4.0.1 and VCN 5.0.0: each IP block's
//! early_init requests its files under `amdgpu/`, as the first trial in a
//! VM logged. The `_kicker` variants are what amdgpu loads instead on boards
//! whose PSP reports a kicker part. The files are linux-firmware's at
//! [`AMDGPU_REVISION`], under AMD's license (`LICENSES/LICENSE.amdgpu`).
use super::*;

/// linux-firmware 20261019, the revision the Realtek files pin.
pub const AMDGPU_REVISION: &str = REALTEK_REVISION;
/// The PCI device IDs of the Radeons whose files [`AMDGPU`] holds.
pub const AMDGPU_DEVICES: [u16; 1] = [0x7550];
/// The target amdgpu's requests are bound under.
pub const AMDGPU_TARGET: Target<'static> = Target {
    driver: "amdgpu",
    hardware: "amd",
    revision: "linux-firmware",
    board: None,
};

const AMDGPU_LICENSE: Artifact = Artifact {
    source: LINUX_FIRMWARE,
    revision: AMDGPU_REVISION,
    member: "LICENSES/LICENSE.amdgpu",
    length: 2938,
    sha256: "572872598565dc3513470de971a32bf9db301f47afeef3636345eadae33b2eee",
};

const fn amdgpu(member: &'static str, length: usize, sha256: &'static str) -> Recipe {
    Recipe {
        image: Artifact {
            source: LINUX_FIRMWARE,
            revision: AMDGPU_REVISION,
            member,
            length,
            sha256,
        },
        license: &AMDGPU_LICENSE,
        consumer: Consumer::Amdgpu,
    }
}

/// Every file amdgpu loads for a Navi 48.
pub const AMDGPU: &[Recipe] = &[
    amdgpu(
        "amdgpu/dcn_4_0_1_dmcub.bin",
        368288,
        "74b567655ac89bfe03389b660e973dc3c067aebd464c6389a071662cfbe6237f",
    ),
    amdgpu(
        "amdgpu/gc_12_0_1_imu.bin",
        132352,
        "b3b301fb636efc77b63ce4d2ced0f90c851d03c19681852faa45598e6f5773fd",
    ),
    amdgpu(
        "amdgpu/gc_12_0_1_imu_kicker.bin",
        132352,
        "874968fa78a0f758f13340b0fbfd0f5edeee124cdd1fb46537bf3ab78bc5c595",
    ),
    amdgpu(
        "amdgpu/gc_12_0_1_me.bin",
        267600,
        "fd5eb0f205a3bb7ec41f42e83a54bdc69151706b02bbee073fa775b8270e4bec",
    ),
    amdgpu(
        "amdgpu/gc_12_0_1_mec.bin",
        259584,
        "83fe5e7bffe639ec9771219cde5c9809689168f769b147c86dfd5a1ca991c1a2",
    ),
    amdgpu(
        "amdgpu/gc_12_0_1_mes.bin",
        643536,
        "306c99718564ff613813b0c15a8e0dd1797488fac01de60702fac0d079c1731d",
    ),
    amdgpu(
        "amdgpu/gc_12_0_1_mes1.bin",
        611920,
        "b873c0e354b94e6ae10831dd883013689a4fb04259c4cd46a9b9ddef4c29ddd5",
    ),
    amdgpu(
        "amdgpu/gc_12_0_1_pfp.bin",
        159616,
        "490bcd26a62e389b3f58acf5c087d324d2324b3188797ff6a36b7a6cdf1ca9ff",
    ),
    amdgpu(
        "amdgpu/gc_12_0_1_rlc.bin",
        150848,
        "6ba4459532246a5c415d3cb33c9b1248294e48f67b827e2accb292a8d1a5c0ec",
    ),
    amdgpu(
        "amdgpu/gc_12_0_1_rlc_kicker.bin",
        150848,
        "6d6a7168e3226b70bffa351f651bfd817375c493def8467a3a53c0ae9b4fb3ff",
    ),
    amdgpu(
        "amdgpu/gc_12_0_1_toc.bin",
        2048,
        "edad661e1c7bd350a86285513fbdeeb33ca4e80c06f1ea7f37d344ba1e2761dd",
    ),
    amdgpu(
        "amdgpu/gc_12_0_1_uni_mes.bin",
        299408,
        "309d60ad4a0210f8a68e27cae88beddb22b5ae900d70848b1e8ca0eac9d1decd",
    ),
    amdgpu(
        "amdgpu/psp_14_0_3_sos.bin",
        435632,
        "28469a0857c813c54a0492423cdf0b0caf757428400036377e19c47e5af62478",
    ),
    amdgpu(
        "amdgpu/psp_14_0_3_sos_kicker.bin",
        435968,
        "1406eb4c19f2e7a11fa4426736f4f4871e4d9f1f7bd7e569e9cf04062edaad69",
    ),
    amdgpu(
        "amdgpu/psp_14_0_3_ta.bin",
        291372,
        "fff8b0b56f21bf1d34c8d1fcf6cc903c2dbb5ef88eb06157c9f08baf95124883",
    ),
    amdgpu(
        "amdgpu/psp_14_0_3_ta_kicker.bin",
        270624,
        "b3dbe9a69dfb3df071856927f686c0c8329f338c7d445bb11535cb7509317093",
    ),
    amdgpu(
        "amdgpu/sdma_7_0_1.bin",
        47104,
        "73c29e1c1714ebc95d2221ba56e187910902891593010653bf9518937e414a59",
    ),
    amdgpu(
        "amdgpu/smu_14_0_3.bin",
        333236,
        "7624724231b0986b59f59705e7c854c482193f6b3b591ebdda58cb62af81c010",
    ),
    amdgpu(
        "amdgpu/smu_14_0_3_kicker.bin",
        327424,
        "c9b0cbef51cc2bfcf28238b904b3e9e089e3e43b226b2bae9d5fcd7be76253e5",
    ),
    amdgpu(
        "amdgpu/vcn_5_0_0.bin",
        440400,
        "9a0ab35bf157c835cbbc4a207ba819ea0111e2a65fe7f8c20910ffff3122f903",
    ),
];

/// The recipe for amdgpu's exact request, under [`AMDGPU_TARGET`].
pub(super) fn select_amdgpu(request: &Request) -> Option<&'static Recipe> {
    if request.target.hardware != AMDGPU_TARGET.hardware
        || request.target.revision != AMDGPU_TARGET.revision
    {
        return None;
    }
    AMDGPU
        .iter()
        .find(|recipe| recipe.image.member == request.name)
}

/// Requires every file amdgpu loads for the Radeon with PCI device ID
/// `device`.
pub fn require_amdgpu(inventory: &mut Inventory, device: u16) -> Result<(), Error> {
    if !AMDGPU_DEVICES.contains(&device) {
        return Err(Error(format!(
            "no pinned amdgpu files for the Radeon {device:04x}"
        )));
    }
    for recipe in AMDGPU {
        inventory.observe(Requirement {
            name: recipe.image.member,
            target: AMDGPU_TARGET,
            minimum_bytes: 1,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_navi_48_requires_every_pinned_file_and_others_none() {
        let mut inventory = Inventory::default();
        require_amdgpu(&mut inventory, 0x7550).unwrap();
        let plan = Plan::select(&inventory).unwrap();
        assert_eq!(plan.selections().len(), AMDGPU.len());
        assert!(require_amdgpu(&mut Inventory::default(), 0x744c).is_err());
    }

    #[test]
    fn requests_under_another_target_find_no_recipe() {
        let request = Request::from(Requirement {
            name: "amdgpu/psp_14_0_3_sos.bin",
            target: Target {
                hardware: "nvidia",
                ..AMDGPU_TARGET
            },
            minimum_bytes: 1,
        });
        assert!(select_amdgpu(&request).is_none());
    }
}

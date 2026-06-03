// SPDX-License-Identifier: MIT

//! Credit to community members for most of the work with notable contributions by:
//!
//! - DualCoder for the original [`vgpu_unlock`](https://github.com/DualCoder/vgpu_unlock)
//! - DualCoder, snowman, Felix, Elec for vGPU profile modification at runtime
//! - NVIDIA for their open-source driver [sources](https://github.com/NVIDIA/open-gpu-kernel-modules)
//! - Arc Compute for their work on Mdev-GPU and GVM documenting more field names in the vGPU
//!   configuration structure

use std::cmp;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{ErrorKind, Write};
use std::mem;
use std::os::raw::{c_int, c_ulong, c_void};
use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::process;
use std::str;

use ctor::ctor;
use libc::RTLD_NEXT;
use parking_lot::Mutex;
use serde::Deserialize;

mod config;
mod dump;
mod format;
mod human_number;
mod ioctl;
mod log;
mod nvidia;
mod to_bytes;
mod utils;
mod uuid;

use crate::config::Config;
use crate::format::WideCharFormat;
use crate::log::{error, info, warning};
use crate::nvidia::ctrl0000vgpu::{
    Nv0000CtrlVgpuCreateDeviceParams, Nv0000CtrlVgpuGetStartDataParams,
    NV0000_CTRL_CMD_VGPU_CREATE_DEVICE, NV0000_CTRL_CMD_VGPU_GET_START_DATA,
};
use crate::nvidia::ctrl0080gpu::{
    Nv0080CtrlGpuGetVirtualizationModeParams, NV0080_CTRL_CMD_GPU_GET_VIRTUALIZATION_MODE,
    NV0080_CTRL_GPU_VIRTUALIZATION_MODE_HOST,
};
use crate::nvidia::ctrl2080bus::{Nv2080CtrlBusGetPciInfoParams, NV2080_CTRL_CMD_BUS_GET_PCI_INFO};
use crate::nvidia::ctrl2080gpu::NV2080_CTRL_CMD_GPU_GET_INFOROM_OBJECT_VERSION;
use crate::nvidia::ctrl9096::NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE;
use crate::nvidia::ctrla081::{
    NvA081CtrlCmdVgpuConfigGetMigrationCapParams, NvA081CtrlVgpuConfigGetVgpuTypeInfoParams,
    NvA081CtrlVgpuInfo, NVA081_CTRL_CMD_VGPU_CONFIG_GET_MIGRATION_CAP,
    NVA081_CTRL_CMD_VGPU_CONFIG_GET_VGPU_TYPE_INFO,
};
use crate::nvidia::ctrla082::{
    NvA082CtrlCmdHostVgpuDeviceGetVgpuTypeInfoParams,
    NVA082_CTRL_CMD_HOST_VGPU_DEVICE_GET_VGPU_TYPE_INFO,
};
use crate::nvidia::error::{
    NV_ERR_BUSY_RETRY, NV_ERR_NOT_SUPPORTED, NV_ERR_OBJECT_NOT_FOUND, NV_OK,
};
use crate::nvidia::nvos::{Nvos54Parameters, NV_ESC_RM_CONTROL};
#[cfg(feature = "proxmox")]
use crate::utils::uuid_to_vmid;
use crate::uuid::Uuid;

static LAST_MDEV_UUID: Mutex<Option<Uuid>> = parking_lot::const_mutex(None);

#[ctor]
static CONFIG: Config = {
    match fs::read_to_string(DEFAULT_CONFIG_PATH) {
        Ok(config) => match toml::from_str::<Config>(&config) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("Failed to decode config: {}", e);

                process::abort();
            }
        },
        Err(e) => {
            if e.kind() != ErrorKind::NotFound {
                eprintln!("Failed to read config: {}", e);
            }

            Default::default()
        }
    }
};

const DEFAULT_CONFIG_PATH: &str = "/etc/vgpu_unlock/config.toml";
const DEFAULT_PROFILE_OVERRIDE_CONFIG_PATH: &str = "/etc/vgpu_unlock/profile_override.toml";

trait VgpuConfigLike {
    fn vgpu_type(&mut self) -> &mut u32;
    fn vgpu_name(&mut self) -> &mut [u8; 64];
    fn vgpu_class(&mut self) -> &mut [u8; 64];
    //fn vgpu_signature(&mut self) -> &mut [u8; 128];
    fn license(&mut self) -> &mut [u8; 128];
    fn max_instance(&mut self) -> &mut u32;
    fn num_heads(&mut self) -> &mut u32;
    fn max_resolution_x(&mut self) -> &mut u32;
    fn max_resolution_y(&mut self) -> &mut u32;
    fn max_pixels(&mut self) -> &mut u32;
    fn frl_config(&mut self) -> &mut u32;
    fn cuda_enabled(&mut self) -> &mut u32;
    fn ecc_supported(&mut self) -> &mut u32;
    fn gpu_instance_size(&mut self) -> &mut u32;
    fn multi_vgpu_supported(&mut self) -> &mut u32;
    fn vdev_id(&mut self) -> &mut u64;
    fn pdev_id(&mut self) -> &mut u64;
    //fn profile_size(&mut self) -> Option<&mut u64>;
    fn fb_length(&mut self) -> &mut u64;
    fn mappable_video_size(&mut self) -> &mut u64;
    fn fb_reservation(&mut self) -> &mut u64;
    fn encoder_capacity(&mut self) -> &mut u32;
    fn bar1_length(&mut self) -> &mut u64;
    fn frl_enable(&mut self) -> &mut u32;
    fn adapter_name(&mut self) -> &mut [u8; 64];
    fn adapter_name_unicode(&mut self) -> &mut [u16; 64];
    fn short_gpu_name_string(&mut self) -> &mut [u8; 64];
    fn licensed_product_name(&mut self) -> &mut [u8; 128];
    //fn vgpu_extra_params(&mut self) -> &mut [u8];
}

macro_rules! impl_trait_fn {
    ($field:ident, $t:ty) => {
        fn $field(&mut self) -> &mut $t {
            &mut self.$field
        }
    };
    ($source_field:ident => $target_field:ident, $t:ty) => {
        fn $target_field(&mut self) -> &mut $t {
            &mut self.$source_field
        }
    };
}
macro_rules! impl_trait_fn_aligned {
    ($field:ident, $t:ty) => {
        fn $field(&mut self) -> &mut $t {
            &mut self.$field.0
        }
    };
}

impl VgpuConfigLike for NvA082CtrlCmdHostVgpuDeviceGetVgpuTypeInfoParams {
    impl_trait_fn!(vgpu_type, u32);
    impl_trait_fn!(vgpu_name, [u8; 64]);
    impl_trait_fn!(vgpu_class, [u8; 64]);
    //impl_trait_fn!(vgpu_signature, [u8; 128]);
    impl_trait_fn!(license, [u8; 128]);
    impl_trait_fn!(max_instance, u32);
    impl_trait_fn!(num_heads, u32);
    impl_trait_fn!(max_resolution_x, u32);
    impl_trait_fn!(max_resolution_y, u32);
    impl_trait_fn!(max_pixels, u32);
    impl_trait_fn!(frl_config, u32);
    impl_trait_fn!(cuda_enabled, u32);
    impl_trait_fn!(ecc_supported, u32);
    impl_trait_fn!(gpu_instance_size, u32);
    impl_trait_fn!(multi_vgpu_supported, u32);
    impl_trait_fn!(vdev_id, u64);
    impl_trait_fn!(pdev_id, u64);

    /*
    fn profile_size(&mut self) -> Option<&mut u64> {
        None
    }
    */

    impl_trait_fn!(fb_length, u64);
    impl_trait_fn!(mappable_video_size, u64);
    impl_trait_fn!(fb_reservation, u64);
    impl_trait_fn!(encoder_capacity, u32);
    impl_trait_fn!(bar1_length, u64);
    impl_trait_fn!(frl_enable, u32);
    impl_trait_fn!(adapter_name, [u8; 64]);
    impl_trait_fn!(adapter_name_unicode, [u16; 64]);
    impl_trait_fn!(short_gpu_name_string, [u8; 64]);
    impl_trait_fn!(licensed_product_name, [u8; 128]);
    //impl_trait_fn!(vgpu_extra_params, [u8]);
}

impl VgpuConfigLike for NvA081CtrlVgpuInfo {
    impl_trait_fn!(vgpu_type, u32);
    impl_trait_fn!(vgpu_name, [u8; 64]);
    impl_trait_fn!(vgpu_class, [u8; 64]);
    //impl_trait_fn!(vgpu_signature, [u8; 128]);
    impl_trait_fn!(license, [u8; 128]);
    impl_trait_fn!(max_instance, u32);
    impl_trait_fn!(num_heads, u32);
    impl_trait_fn!(max_resolution_x, u32);
    impl_trait_fn!(max_resolution_y, u32);
    impl_trait_fn!(max_pixels, u32);
    impl_trait_fn!(frl_config, u32);
    impl_trait_fn!(cuda_enabled, u32);
    impl_trait_fn!(ecc_supported, u32);
    impl_trait_fn!(gpu_instance_size, u32);
    impl_trait_fn!(multi_vgpu_supported, u32);
    impl_trait_fn_aligned!(vdev_id, u64);
    impl_trait_fn_aligned!(pdev_id, u64);

    /*
    fn profile_size(&mut self) -> Option<&mut u64> {
        Some(&mut self.profile_size.0)
    }
    */

    impl_trait_fn_aligned!(fb_length, u64);
    impl_trait_fn_aligned!(mappable_video_size, u64);
    impl_trait_fn_aligned!(fb_reservation, u64);
    impl_trait_fn!(encoder_capacity, u32);
    impl_trait_fn_aligned!(bar1_length, u64);
    impl_trait_fn!(frl_enable, u32);
    impl_trait_fn!(adapter_name, [u8; 64]);
    impl_trait_fn!(adapter_name_unicode, [u16; 64]);
    impl_trait_fn!(short_gpu_name_string, [u8; 64]);
    impl_trait_fn!(licensed_product_name, [u8; 128]);
    //impl_trait_fn!(vgpu_extra_params, [u8]);
}

#[derive(Deserialize)]
struct ProfileOverridesConfig {
    #[serde(default)]
    custom: CustomOverrides,
    #[serde(default)]
    profile: HashMap<String, VgpuProfileOverride>,
    #[serde(default)]
    mdev: HashMap<String, VgpuProfileOverride>,
    #[cfg(feature = "proxmox")]
    #[serde(default)]
    vm: HashMap<String, VgpuProfileOverride>,
}

#[derive(Default, Deserialize)]
struct CustomOverrides {
    #[serde(default, deserialize_with = "crate::config::deserialize_optional_pci_id")]
    spoofed_devid: Option<u32>,
    #[serde(default, deserialize_with = "crate::config::deserialize_optional_pci_id")]
    spoofed_subsysid: Option<u32>,
}

#[derive(Default, Deserialize)]
struct VgpuProfileOverride {
    gpu_type: Option<u32>,
    card_name: Option<String>,
    vgpu_type: Option<String>,
    features: Option<String>,
    max_instances: Option<u32>,
    num_displays: Option<u32>,
    display_width: Option<u32>,
    display_height: Option<u32>,
    max_pixels: Option<u32>,
    frl_config: Option<u32>,
    cuda_enabled: Option<u32>,
    ecc_supported: Option<u32>,
    mig_instance_size: Option<u32>,
    multi_vgpu_supported: Option<u32>,
    pci_id: Option<u64>,
    pci_device_id: Option<u64>,
    #[serde(default, with = "human_number")]
    framebuffer: Option<u64>,
    #[serde(default, with = "human_number")]
    mappable_video_size: Option<u64>,
    #[serde(default, with = "human_number")]
    framebuffer_reservation: Option<u64>,
    encoder_capacity: Option<u32>,
    bar1_length: Option<u64>,
    frl_enabled: Option<u32>,
    adapter_name: Option<String>,
    short_gpu_name: Option<String>,
    license_type: Option<String>,
}

fn check_size(name: &str, actual_size: usize, expected_size: usize) -> bool {
    if actual_size != expected_size {
        error!(
            "Parameters size for {} was {} bytes, expected {} bytes",
            name, actual_size, expected_size
        );

        false
    } else {
        true
    }
}

/// # Safety
///
/// This is actually unsafe since `ioctl` is variadic. All the `ioctl` calls in the
/// 460.32.04 `nvidia-vgpu-mgr` and `nvidia-vgpud` binaries use only one argument.
#[no_mangle]
pub unsafe extern "C" fn ioctl(fd: RawFd, request: c_ulong, argp: *mut c_void) -> c_int {
    static mut IOCTL_FN_PTR: Option<unsafe extern "C" fn(RawFd, c_ulong, ...) -> c_int> = None;

    //info!("ioctl({}, {}, {:?})", fd, request, data);

    let next_ioctl = match IOCTL_FN_PTR {
        Some(func) => func,
        None => {
            let next_ioctl = mem::transmute(libc::dlsym(RTLD_NEXT, b"ioctl\0".as_ptr() as _));

            IOCTL_FN_PTR = mem::transmute(next_ioctl);

            next_ioctl
        }
    };

    let ret = next_ioctl(fd, request, argp);

    if request != NV_ESC_RM_CONTROL {
        // Not a call we care about.
        return ret;
    }

    if ret < 0 {
        // Call failed.
        return ret;
    }

    // Safety: NVIDIA's driver itself uses `sizeof` when calculating the ioctl number and so does
    // this hook so the structure passed in should be of the correct size.
    let io_data: &mut Nvos54Parameters = &mut *argp.cast();

    if io_data.status == NV_ERR_BUSY_RETRY {
        // Driver will try again.
        return ret;
    }

    //info!("{:#x?}", io_data);

    macro_rules! check_size {
        ($name:ident, $expected_type:ty) => {
            check_size(
                stringify!($name),
                io_data.params_size as usize,
                mem::size_of::<$expected_type>(),
            )
        };
        ($name:ident, size: $expected_size:expr) => {
            check_size(
                stringify!($name),
                io_data.params_size as usize,
                $expected_size,
            )
        };
    }

    match io_data.cmd {
        NV2080_CTRL_CMD_BUS_GET_PCI_INFO
            if check_size!(
                NV2080_CTRL_CMD_BUS_GET_PCI_INFO,
                Nv2080CtrlBusGetPciInfoParams
            ) && CONFIG.unlock =>
        {
            let params: &mut Nv2080CtrlBusGetPciInfoParams = &mut *io_data.params.cast();

            let orig_device_id = params.pci_device_id;
            let orig_sub_system_id = params.pci_sub_system_id;

            let actual_device_id = (orig_device_id & 0xffff0000) >> 16;
            let actual_sub_system_id = (orig_sub_system_id & 0xffff0000) >> 16;

            let (mut spoofed_devid, mut spoofed_subsysid) = match actual_device_id {
                //T4
                0x1EB8 => {
                    (actual_device_id, actual_sub_system_id)
                }
                // Maxwell
                0x1340..=0x137F | 0x1380..=0x13BF => {
                //GM108 | GM107
                    // A5500
                    (0x2233, 0x165a)
                }
                // Maxwell 2.0
                0x17c0..=0x17FF | 0x13c0..=0x13FF | 0x1400..=0x143F => {
                //GM200 | GM204 | GM206
                    // A5500
                    (0x2233, 0x165a)
                }
                // Pascal
                0x15C0..=0x15FF | 0x1B00..=0x1B3F | 0x1B80..=0x1BBF | 0x1C00..=0x1C3F | 0x1C80..=0x1CBF | 0x1D00..=0x1D3F => {
                //GP100 | GP102 | GP104 | GP106 | GP107 | GP108
                    // A5500
                    (0x2233, 0x165a)
                }
                // GV100 Volta
                //
                // 0x1d81 = TITAN V
                // 0x1dba = Quadro GV100 32GB
                0x1D80..=0x1dba => {
                //ARCH_VOLTA
                    // A5500
                    (0x2233, 0x165a)
                }
                // Turing
                0x1E00..=0x1E3F | 0x1E80..=0x1EBF | 0x1F00..=0x1F3F | 0x2180..=0x21BF | 0x1F80..=0x1FBF => {
                    // A5500
                    (0x2233, 0x165a)
                }
                // Ampere
                0x2200..=0x2600 => {
                    // RTX A6000
                    (0x2230, actual_sub_system_id)
                }
                _ => (actual_device_id, actual_sub_system_id),
            };

            let profile_override_config = load_profile_overrides().ok();

            if let Some(config_spoofed_devid) = profile_override_config
                .as_ref()
                .and_then(|config| config.custom.spoofed_devid)
                .or(CONFIG.spoofed_devid)
            {
                info!(
                    "Overriding spoofed PCI device ID: {:#06x} -> {:#06x}",
                    spoofed_devid, config_spoofed_devid
                );
                spoofed_devid = config_spoofed_devid;
            }

            if let Some(config_spoofed_subsysid) = profile_override_config
                .as_ref()
                .and_then(|config| config.custom.spoofed_subsysid)
                .or(CONFIG.spoofed_subsysid)
            {
                info!(
                    "Overriding spoofed PCI subsystem ID: {:#06x} -> {:#06x}",
                    spoofed_subsysid, config_spoofed_subsysid
                );
                spoofed_subsysid = config_spoofed_subsysid;
            }

            params.pci_device_id = (orig_device_id & 0xffff) | (spoofed_devid << 16);
            params.pci_sub_system_id = (orig_sub_system_id & 0xffff) | (spoofed_subsysid << 16);
        }
        NV0080_CTRL_CMD_GPU_GET_VIRTUALIZATION_MODE
            if check_size!(
                NV0080_CTRL_CMD_GPU_GET_VIRTUALIZATION_MODE,
                Nv0080CtrlGpuGetVirtualizationModeParams
            ) && CONFIG.unlock =>
        {
            let params: &mut Nv0080CtrlGpuGetVirtualizationModeParams = &mut *io_data.params.cast();

            // Set device type to vGPU capable.
            params.virtualization_mode = NV0080_CTRL_GPU_VIRTUALIZATION_MODE_HOST;
        }
        NVA081_CTRL_CMD_VGPU_CONFIG_GET_MIGRATION_CAP
            if check_size!(
                NVA081_CTRL_CMD_VGPU_CONFIG_GET_MIGRATION_CAP,
                NvA081CtrlCmdVgpuConfigGetMigrationCapParams
            ) && CONFIG.unlock_migration =>
        {
            let params: &mut NvA081CtrlCmdVgpuConfigGetMigrationCapParams =
                &mut *io_data.params.cast();

            params.migration_cap = 1;
        }
        _ => {}
    }

    if io_data.status == NV_OK {
        match io_data.cmd {
            NV0000_CTRL_CMD_VGPU_GET_START_DATA
                if check_size!(
                    NV0000_CTRL_CMD_VGPU_GET_START_DATA,
                    Nv0000CtrlVgpuGetStartDataParams
                ) =>
            {
                let config: &Nv0000CtrlVgpuGetStartDataParams = &*io_data.params.cast();
                info!("{:#?}", config);

                *LAST_MDEV_UUID.lock() = Some(config.mdev_uuid);
            }
            NV0000_CTRL_CMD_VGPU_CREATE_DEVICE
                if check_size!(
                    NV0000_CTRL_CMD_VGPU_CREATE_DEVICE,
                    Nv0000CtrlVgpuCreateDeviceParams
                ) =>
            {
                // 17.0 driver provides mdev uuid as vgpu_name in this command
                let params: &mut Nv0000CtrlVgpuCreateDeviceParams = &mut *io_data.params.cast();
                info!("{:#?}", params);

                *LAST_MDEV_UUID.lock() = Some(params.vgpu_name);
            }
            NVA081_CTRL_CMD_VGPU_CONFIG_GET_VGPU_TYPE_INFO
                if check_size!(
                    NVA081_CTRL_CMD_VGPU_CONFIG_GET_VGPU_TYPE_INFO,
                    NvA081CtrlVgpuConfigGetVgpuTypeInfoParams
                ) =>
            {
                let params: &mut NvA081CtrlVgpuConfigGetVgpuTypeInfoParams =
                    &mut *io_data.params.cast();
                info!("{:#?}", params);

                if !handle_profile_override(&mut params.vgpu_type_info) {
                    error!("Failed to apply profile override");
                    return -1;
                }
            }
            NVA082_CTRL_CMD_HOST_VGPU_DEVICE_GET_VGPU_TYPE_INFO
                if check_size!(
                    NVA082_CTRL_CMD_HOST_VGPU_DEVICE_GET_VGPU_TYPE_INFO,
                    NvA082CtrlCmdHostVgpuDeviceGetVgpuTypeInfoParams
                ) =>
            {
                let params: &mut NvA082CtrlCmdHostVgpuDeviceGetVgpuTypeInfoParams =
                    &mut *io_data.params.cast();
                info!("{:#?}", params);

                if !handle_profile_override(params) {
                    error!("Failed to apply profile override");
                    return -1;
                }
            }
            _ => {}
        }
    }

    if io_data.status != NV_OK {
        // Things seems to work fine even if some operations that fail result in failed assertions.
        // So here we change the status value for these cases to cleanup the logs for
        // `nvidia-vgpu-mgr`.
        if io_data.cmd == 0xa0820104 || io_data.cmd == NV9096_CTRL_CMD_GET_ZBC_CLEAR_TABLE {
            io_data.status = NV_OK;
        } else {
            error!("cmd: {:#x} failed.", io_data.cmd);
        }
    }

    // Workaround for some Maxwell cards not supporting reading inforom.
    if io_data.cmd == NV2080_CTRL_CMD_GPU_GET_INFOROM_OBJECT_VERSION
        && io_data.status == NV_ERR_NOT_SUPPORTED
    {
        io_data.status = NV_ERR_OBJECT_NOT_FOUND;
    }

    ret
}

fn load_overrides() -> Result<String, bool> {
    let config_path = match env::var_os("VGPU_UNLOCK_PROFILE_OVERRIDE_CONFIG_PATH") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(DEFAULT_PROFILE_OVERRIDE_CONFIG_PATH),
    };

    let config_overrides = match fs::read_to_string(&config_path) {
        Ok(data) => data,
        Err(e) => {
            if e.kind() == ErrorKind::NotFound {
                error!("Config file '{}' not found", config_path.display());
                return Err(true);
            }

            error!("Failed to read '{}': {}", config_path.display(), e);
            return Err(false);
        }
    };

    Ok(config_overrides)
}

fn load_profile_overrides() -> Result<ProfileOverridesConfig, bool> {
    let config_overrides = load_overrides()?;

    match toml::from_str(&config_overrides) {
        Ok(config) => Ok(config),
        Err(e) => {
            error!("Failed to decode config: {}", e);
            Err(false)
        }
    }
}

fn handle_profile_override<C: VgpuConfigLike>(config: &mut C) -> bool {
    let vgpu_type = format!("nvidia-{}", config.vgpu_type());
    let mdev_uuid = LAST_MDEV_UUID.lock().clone();
    let config_overrides = load_profile_overrides();

    match &config_overrides {
        Ok(config_overrides) => {
            if let Some(config_override) = config_overrides.profile.get(vgpu_type.as_str()) {
                info!("Applying profile {} overrides", vgpu_type);

                if !apply_profile_override(config, &vgpu_type, config_override) {
                    return false;
                }
            }
            if let Some(mdev_uuid) = mdev_uuid.map(|uuid| uuid.to_string()) {
                if let Some(config_override) = config_overrides.mdev.get(mdev_uuid.as_str()) {
                    info!("Applying mdev UUID {} profile overrides", mdev_uuid);

                    if !apply_profile_override(config, &vgpu_type, config_override) {
                        return false;
                    }
                }
            }
        }
        Err(true) => {}
        Err(false) => return false,
    }

    #[cfg(feature = "proxmox")]
    if let Some(vmid) = mdev_uuid.and_then(uuid_to_vmid) {
        let vmid = vmid.to_string();
        info!("VMID {} profile", vmid);
        if let Ok(config_overrides) = &config_overrides {
            if let Some(config_override) = config_overrides.vm.get(vmid.as_str()) {
                info!("Applying proxmox VMID {} profile overrides", vmid);

                if !apply_profile_override(config, &vgpu_type, config_override) {
                    return false;
                }
            }
        }

        if let Some(config_override) = load_proxmox_vm_profile_override(vmid.as_str()) {
            if !apply_proxmox_profile_override(config, &vgpu_type, config_override) {
                return false;
            }
        }
    }

    true
}

#[cfg(feature = "proxmox")]
fn load_proxmox_vm_profile_override(vmid: &str) -> Option<VgpuProfileOverride> {
    let config_path = PathBuf::from(format!("/etc/pve/qemu-server/{}.conf", vmid));
    let config_data = match fs::read_to_string(&config_path) {
        Ok(data) => data,
        Err(e) => {
            if e.kind() != ErrorKind::NotFound {
                error!(
                    "Failed to read Proxmox VM config '{}': {}",
                    config_path.display(),
                    e
                );
            }

            return None;
        }
    };

    let mut selected: Option<(u32, String, VgpuProfileOverride)> = None;
    let mut eligible_count = 0;

    for line in config_data.lines() {
        let line = line.trim();

        if line.starts_with('[') {
            break;
        }

        if let Some((hostpci_index, config_override)) = parse_proxmox_hostpci_override(line) {
            eligible_count += 1;

            if selected
                .as_ref()
                .map(|(selected_index, _, _)| hostpci_index < *selected_index)
                .unwrap_or(true)
            {
                selected = Some((hostpci_index, line.to_string(), config_override));
            }
        }
    }

    if eligible_count > 1 {
        warning!(
            "Multiple eligible Proxmox vGPU hostpci entries found for VMID {}; using the lowest hostpci index",
            vmid
        );
    }

    selected.map(|(_, line, config_override)| {
        info!("Applying Proxmox VMID {} hostpci config: {}", vmid, line);
        config_override
    })
}

#[cfg(feature = "proxmox")]
fn parse_proxmox_hostpci_override(line: &str) -> Option<(u32, VgpuProfileOverride)> {
    let (key, value) = split_once(line, ':')?;
    let hostpci_index = parse_hostpci_index(key.trim())?;

    let value = value.trim();
    let mut has_nvidia_mdev = false;
    let mut vgpu_unlock = false;
    let mut config_override = VgpuProfileOverride::default();

    for entry in value.split(',') {
        let entry = entry.trim();
        let (name, value) = match split_once(entry, '=') {
            Some(parts) => parts,
            None => continue,
        };

        let name = name.trim();
        let value = value.trim();

        match name {
            "mdev" => has_nvidia_mdev = value.starts_with("nvidia-"),
            "vgpu-unlock" => vgpu_unlock = parse_proxmox_bool(value).unwrap_or(false),
            "framebuffer" => match value.parse::<u64>() {
                Ok(framebuffer_mb) => match framebuffer_mb.checked_mul(1024 * 1024) {
                    Some(framebuffer) => config_override.framebuffer = Some(framebuffer),
                    None => error!("Proxmox hostpci framebuffer '{}' MB is too large", value),
                },
                Err(e) => error!("Invalid Proxmox hostpci framebuffer '{}': {}", value, e),
            },
            "cuda" => match parse_proxmox_bool(value) {
                Some(cuda) => config_override.cuda_enabled = Some(cuda as u32),
                None => error!("Invalid Proxmox hostpci cuda '{}'", value),
            },
            "frl" => match parse_proxmox_bool(value) {
                Some(frl) => config_override.frl_enabled = Some(frl as u32),
                None => error!("Invalid Proxmox hostpci frl '{}'", value),
            },
            "vgpu-type" => config_override.vgpu_type = Some(value.to_string()),
            _ => {}
        }
    }

    if vgpu_unlock && has_nvidia_mdev {
        Some((hostpci_index, config_override))
    } else {
        None
    }
}

#[cfg(feature = "proxmox")]
fn parse_hostpci_index(key: &str) -> Option<u32> {
    let index = key.strip_prefix("hostpci")?;

    if index.is_empty() {
        return None;
    }

    index.parse().ok()
}

#[cfg(feature = "proxmox")]
fn parse_proxmox_bool(value: &str) -> Option<bool> {
    match value {
        "1" | "on" | "yes" | "true" => Some(true),
        "0" | "off" | "no" | "false" => Some(false),
        _ => None,
    }
}

#[cfg(feature = "proxmox")]
fn split_once(value: &str, delimiter: char) -> Option<(&str, &str)> {
    let mut parts = value.splitn(2, delimiter);
    let key = parts.next()?;
    let value = parts.next()?;

    Some((key, value))
}

#[cfg(feature = "proxmox")]
fn apply_proxmox_profile_override<C: VgpuConfigLike>(
    config: &mut C,
    vgpu_type: &str,
    mut config_override: VgpuProfileOverride,
) -> bool {
    // 0x6000000 bytes = 96 MiB. Keep at least this much framebuffer reservation.
    const MIN_FRAMEBUFFER_RESERVATION: u64 = 0x6000000;

    if let Some(total_framebuffer) = config_override.framebuffer {
        let mut framebuffer_reservation = *config.fb_reservation();

        if total_framebuffer < framebuffer_reservation {
            if total_framebuffer < MIN_FRAMEBUFFER_RESERVATION {
                error!(
                    "Invalid Proxmox framebuffer override for {}: requested total framebuffer {} bytes is smaller than minimum framebuffer_reservation {} bytes; computed framebuffer would be negative",
                    vgpu_type,
                    total_framebuffer,
                    MIN_FRAMEBUFFER_RESERVATION
                );

                return false;
            }

            info!(
                "Reducing Proxmox framebuffer_reservation for {}: {} bytes -> {} bytes",
                vgpu_type,
                framebuffer_reservation,
                MIN_FRAMEBUFFER_RESERVATION
            );

            framebuffer_reservation = MIN_FRAMEBUFFER_RESERVATION;
            config_override.framebuffer_reservation = Some(framebuffer_reservation);
        }

        let framebuffer = total_framebuffer - framebuffer_reservation;

        info!(
            "Proxmox framebuffer calculation for {}: requested total framebuffer {} bytes - framebuffer_reservation {} bytes = framebuffer {} bytes",
            vgpu_type,
            total_framebuffer,
            framebuffer_reservation,
            framebuffer
        );

        config_override.framebuffer = Some(framebuffer);
    }

    apply_profile_override(config, vgpu_type, &config_override)
}

fn apply_profile_override<C: VgpuConfigLike>(
    config: &mut C,
    vgpu_type: &str,
    config_override: &VgpuProfileOverride,
) -> bool {
    macro_rules! patch_msg {
        ($target_field:ident, $value:expr) => {
            info!(
                "Patching {}/{}: {} -> {}",
                vgpu_type,
                stringify!($target_field),
                config.$target_field(),
                $value
            );
        };
        ($target_field:ident, $preprocess:expr, $value:expr) => {
            info!(
                "Patching {}/{}: {} -> {}",
                vgpu_type,
                stringify!($target_field),
                $preprocess(config.$target_field()),
                $value
            );
        };
    }
    macro_rules! error_too_long {
        ($target_field:ident, $value:expr) => {
            error!(
                "Patching {}/{}: value '{}' is too long",
                vgpu_type,
                stringify!($target_field),
                $value
            );

            return false;
        };
    }

    macro_rules! handle_override {
        // Override entrypoint when the same field name is used as the source and target without
        // an explicit `=>`.
        (
            class: $class:ident,
            source_field: $field:ident,
        ) => {
            handle_override! {
                class: $class,
                source_field: $field,
                target_field: $field,
            }
        };

        // Override entrypoint when both the source and target field names are defined explicitly.
        (
            class: $class:ident,
            source_field: $source_field:ident,
            target_field: $target_field:ident,
        ) => {
            if let Some(value) = config_override.$source_field.as_ref() {
                handle_override! {
                    class: $class,
                    value: value,
                    source_field: $source_field,
                    target_field: $target_field,
                }
            }
        };

        // The following are override handlers for each field class type (`bool`, `copy`, `str`,
        // and `wide_str`).
        (
            class: bool,
            value: $value:ident,
            source_field: $source_field:ident,
            target_field: $target_field:ident,
        ) => {
            let $value = cmp::max(cmp::min(*$value, 1), 0);

            patch_msg!($target_field, $value);

            *config.$target_field() = $value;
        };
        (
            class: copy,
            value: $value:ident,
            source_field: $source_field:ident,
            target_field: $target_field:ident,
        ) => {
            patch_msg!($target_field, $value);

            *config.$target_field() = *$value;
        };
        (
            class: str,
            value: $value:ident,
            source_field: $source_field:ident,
            target_field: $target_field:ident,
        ) => {
            let value_bytes = $value.as_bytes();

            // Use `len - 1` to account for the required NULL terminator.
            if value_bytes.len() > config.$target_field().len() - 1 {
                error_too_long!($target_field, $value);
            } else {
                patch_msg!($target_field, utils::from_c_str, $value);

                // Zero out the field first.
                // (`fill` was stabilized in Rust 1.50, but Debian Bullseye ships with 1.48)
                for v in config.$target_field().iter_mut() {
                    *v = 0;
                }

                // Write the string bytes.
                let _ = config.$target_field()[..].as_mut().write_all(value_bytes);
            }
        };
        (
            class: wide_str,
            value: $value:ident,
            source_field: $source_field:ident,
            target_field: $target_field:ident,
        ) => {
            // Use `len - 1` to account for the required NULL terminator.
            if $value.encode_utf16().count() > config.$target_field().len() - 1 {
                error_too_long!($target_field, $value);
            } else {
                patch_msg!($target_field, WideCharFormat, $value);

                // Zero out the field first.
                // (`fill` was stabilized in Rust 1.50, but Debian Bullseye ships with 1.48)
                for v in config.$target_field().iter_mut() {
                    *v = 0;
                }

                // Write the string bytes.
                for (v, ch) in config.$target_field()[..]
                    .iter_mut()
                    .zip($value.encode_utf16().chain(Some(0)))
                {
                    *v = ch;
                }
            }
        };
    }
    macro_rules! handle_overrides {
        (
            $($class:ident: [
                $($source_field:ident $(=> $target_field:ident)?),*$(,)?
            ]),*$(,)?
        ) => {
            $(
                $(
                    handle_override! {
                        class: $class,
                        source_field: $source_field,
                        $(target_field: $target_field,)?
                    }
                )*
            )*
        };
    }

    // While the following could be done with fewer branches, I wanted the log statements to be in
    // field order.

    handle_overrides! {
        copy: [
            gpu_type => vgpu_type,
        ],
        str: [
            card_name => vgpu_name,
            vgpu_type => vgpu_class,
            features => license,
        ],
        copy: [
            max_instances => max_instance,
            num_displays => num_heads,
            display_width => max_resolution_x,
            display_height => max_resolution_y,
            max_pixels,
            frl_config,
        ],
        bool: [
            cuda_enabled,
            ecc_supported,
        ],
        copy: [
            mig_instance_size => gpu_instance_size,
        ],
        bool: [
            multi_vgpu_supported,
        ],
        copy: [
            pci_id => vdev_id,
            pci_device_id => pdev_id,
            framebuffer => fb_length,
            mappable_video_size,
            framebuffer_reservation => fb_reservation,
            encoder_capacity,
            bar1_length,
        ],
        bool: [
            frl_enabled => frl_enable,
        ],
        str: [
            adapter_name,
        ],
        wide_str: [
            adapter_name => adapter_name_unicode,
        ],
        str: [
            short_gpu_name => short_gpu_name_string,
            license_type => licensed_product_name,
        ],
    }

    true
}

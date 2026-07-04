//! Embedded compiled-Rust userspace images (Layer 5.1). The VFS service and a
//! client, each linked at USER_CODE_VA into its own blob. FIRST images with
//! real .rodata (parser literals / path strings) -> first live exercise of the
//! loader's rodata region path.

use crate::process::UserImage;
use crate::user_layout;

static VFS_BLOB:    &[u8] = include_bytes!("vfs.bin");
static CLIENT_BLOB: &[u8] = include_bytes!("client.bin");
static CLIENT_B_BLOB: &[u8] = include_bytes!("client_b.bin");

pub fn vfs_image() -> UserImage {
    use user_layout::vfs::*;
    UserImage { blob: VFS_BLOB,
        text_va:TEXT_VA, text_len:TEXT_LEN, rodata_va:RODATA_VA, rodata_len:RODATA_LEN,
        data_va:DATA_VA, data_len:DATA_LEN, bss_va:BSS_VA, bss_len:BSS_LEN, entry:TEXT_VA }
}

pub fn client_b_image() -> UserImage {
    use user_layout::client_b::*;
    UserImage { blob: CLIENT_B_BLOB,
        text_va:TEXT_VA, text_len:TEXT_LEN, rodata_va:RODATA_VA, rodata_len:RODATA_LEN,
        data_va:DATA_VA, data_len:DATA_LEN, bss_va:BSS_VA, bss_len:BSS_LEN, entry:TEXT_VA }
}

pub fn client_image() -> UserImage {
    use user_layout::client::*;
    UserImage { blob: CLIENT_BLOB,
        text_va:TEXT_VA, text_len:TEXT_LEN, rodata_va:RODATA_VA, rodata_len:RODATA_LEN,
        data_va:DATA_VA, data_len:DATA_LEN, bss_va:BSS_VA, bss_len:BSS_LEN, entry:TEXT_VA }
}

use crate::{
    cpp_string::{ResourceLocation, StackString},
    LockResultExt,
};
use cxx::CxxString;
// use ndk::asset::AssetManager;
use std::{
    io::{self, Cursor, Read, Seek, Write},
    mem::transmute,
    ops::{Deref, DerefMut},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    pin::Pin,
};

pub enum BufferCursor {
    Vec(Cursor<Vec<u8>>),
    Cxx(Cursor<StackString>),
}
impl Read for BufferCursor {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Vec(v) => v.read(buf),
            Self::Cxx(cxx) => cxx.read(buf),
        }
    }
}
impl Seek for BufferCursor {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        match self {
            Self::Vec(v) => v.seek(pos),
            Self::Cxx(cxx) => cxx.seek(pos),
        }
    }
}
impl BufferCursor {
    pub fn position(&self) -> u64 {
        match self {
            Self::Vec(v) => v.position(),
            Self::Cxx(cxx) => cxx.position(),
        }
    }
    pub fn get_ref(&self) -> &[u8] {
        match self {
            Self::Vec(v) => v.get_ref(),
            Self::Cxx(cxx) => cxx.get_ref().as_ref(),
        }
    }
}
macro_rules! folder_list {
    ($( apk: $apk_folder:literal -> pack: $pack_folder:expr),
        *,
    ) => {
        [
            $(($apk_folder, $pack_folder)),*,
        ]
    }
}

pub struct FileLoader {
    pub last_buffer: Option<Buffer>,
}
impl FileLoader {
    pub fn new() -> Self {
        Self { last_buffer: None }
    }
    pub fn get_file(&mut self, path: &Path) -> Option<Buffer> {
        let stripped = path.strip_prefix("assets/").unwrap_or(path);
        // if let Some(mut cache) = self.last_buffer.take_if(|c| c.name == path) {
        //     log::info!("Cache hit!: {:#?}", path);
        //     cache
        //         .rewind()
        //         .expect("Unable to rewind in a memory buffer?, impossible");
        //     return Some(cache);
        // }
        let replacement_list = folder_list! {
            apk: "gui/dist/hbui/" -> pack: "hbui/",
            apk: "skin_packs/persona/" -> pack: "persona/",
            apk: "renderer/" -> pack: "renderer/",
            apk: "resource_packs/vanilla/cameras/" -> pack: "vanilla_cameras/",
        };
        for replacement in replacement_list {
            // Remove the prefix we want to change
            if let Ok(file) = stripped.strip_prefix(replacement.0) {
                let mut resource_loc = ResourceLocation::new();
                let mut cpppath = ResourceLocation::get_path(&mut resource_loc);
                opt_path_join(cpppath.as_mut(), &[Path::new(replacement.1), file]);
                let packm = crate::PACKM_OBJ.lock().ignore_poison();
                let Some(packm) = packm.as_ref() else {
                    log::error!("ResourcePackManager ptr is null");
                    return None;
                };
                let Some(stack_str) = packm.load_resource(resource_loc) else {
                    log::info!("Cannot find file: {}", cpppath.as_ref());
                    return None;
                };
                log::info!("Loaded ResourcePack file: {}", cpppath.as_ref());
                let mut data = stack_str.as_ref().to_vec();
                patch_arm_shaders(&mut data);
                let buffer = BufferCursor::Vec(Cursor::new(data));
                let cache = Buffer::new(path.to_path_buf(), buffer);
                return Some(cache);
            }
        }
        None
    }
}
pub struct Buffer {
    name: PathBuf,
    object: BufferCursor,
}
impl Buffer {
    fn new(name: PathBuf, object: BufferCursor) -> Self {
        Self { name, object }
    }
}
impl Deref for Buffer {
    type Target = BufferCursor;
    fn deref(&self) -> &Self::Target {
        &self.object
    }
}
impl DerefMut for Buffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.object
    }
}

// This lint is not really applicable
#[allow(clippy::unused_io_amount)]
/// Join paths directly into a c++ string
fn opt_path_join(mut bytes: Pin<&mut CxxString>, paths: &[&Path]) {
    let total_len: usize = paths.iter().map(|p| p.as_os_str().len()).sum();
    bytes.as_mut().reserve(total_len);
    let mut writer = bytes;
    for path in paths {
        let osstr = path.as_os_str().as_bytes();
        writer
            .write(osstr)
            .expect("Error while writing path to stack path");
    }
}
pub struct ResourcePackManager(*mut libc::c_void);
// Technically we can pass this everywhere as its just a handle basically
//unsafe impl Sync for ResourcePackManager {}
unsafe impl Send for ResourcePackManager {}
impl ResourcePackManager {
    pub fn wrap(ptr: *mut libc::c_void) -> Self {
        Self(ptr)
    }
    pub fn load_resource(&self, loc: ResourceLocation) -> Option<StackString> {
        let vptr = unsafe { *transmute::<*mut libc::c_void, *mut *mut *const u8>(self.0) };
        let loadfn = unsafe {
            transmute::<
                *const u8,
                unsafe extern "C" fn(
                    *mut libc::c_void,
                    ResourceLocation,
                    Pin<&mut CxxString>,
                ) -> bool,
            >(*vptr.offset(2))
        };
        let mut cxx_storage = StackString::new();
        let mut cxx_ptr = unsafe { cxx_storage.init("") };
        unsafe { loadfn(self.0, loc, cxx_ptr.as_mut()) };
        if cxx_ptr.is_empty() {
            None
        } else {
            Some(cxx_storage)
        }
    }
}

fn patch_arm_shaders(content: &mut Vec<u8>) {
    let tag = b"//TAG: FAST_PATH_MALI";
    if content.starts_with(tag) {
        let headers = b"\n#extension GL_ARM_shader_framebuffer_fetch : require\n                       #extension GL_EXT_shader_pixel_local_storage : require\n                       #extension GL_ARM_increased_rt : enable\n";
        content.splice(tag.len()..tag.len(), headers.iter().cloned());
    }
}

pub fn aplicar_patch_sonoro(bytes: &[u8]) -> Vec<u8> {
    let tag = b"//TAG: FAST_PATH_MALI";
    let mut novo_conteudo = bytes.to_vec();
    if novo_conteudo.starts_with(tag) {
        let headers = b"\n#extension GL_ARM_shader_framebuffer_fetch : require\n#extension GL_EXT_shader_pixel_local_storage : require\n#extension GL_ARM_increased_rt : enable\n";
        novo_conteudo.splice(tag.len()..tag.len(), headers.iter().cloned());
    }
    novo_conteudo
}

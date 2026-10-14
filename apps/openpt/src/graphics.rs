use eframe::{
    NativeOptions,
    egui_wgpu::WgpuSetup,
    wgpu::{Backends, Dx12Compiler},
};

pub fn configure(options: &mut NativeOptions, backend_override: Option<Backends>, compiler_override: Option<Dx12Compiler>) {
    if let WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        create.instance_descriptor.backends = backend_override.unwrap_or(Backends::DX12);
        create.instance_descriptor.backend_options.dx12.shader_compiler = compiler_override.unwrap_or(Dx12Compiler::Fxc);
        log::info!(
            "Windows graphics: backends={:?}, DX12 compiler={:?}",
            create.instance_descriptor.backends,
            create.instance_descriptor.backend_options.dx12.shader_compiler
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured_backends(backend_override: Option<Backends>) -> Backends {
        let mut options = NativeOptions::default();
        configure(&mut options, backend_override, None);
        let WgpuSetup::CreateNew(create) = options.wgpu_options.wgpu_setup else {
            panic!("default native options must create a wgpu instance");
        };
        create.instance_descriptor.backends
    }

    #[test]
    fn windows_default_initializes_only_dx12() {
        let WgpuSetup::CreateNew(eframe_default) = NativeOptions::default().wgpu_options.wgpu_setup else {
            panic!("default native options must create a wgpu instance");
        };
        assert!(eframe_default.instance_descriptor.backends.contains(Backends::GL));
        assert_eq!(configured_backends(None), Backends::DX12);
    }

    #[test]
    fn windows_default_uses_fxc_instead_of_searching_for_dxc() {
        let mut options = NativeOptions::default();
        if let WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
            create.instance_descriptor.backend_options.dx12.shader_compiler = Dx12Compiler::Auto;
        }
        configure(&mut options, None, None);
        let WgpuSetup::CreateNew(create) = options.wgpu_options.wgpu_setup else {
            panic!("default native options must create a wgpu instance");
        };
        assert!(matches!(create.instance_descriptor.backend_options.dx12.shader_compiler, Dx12Compiler::Fxc));
    }

    #[test]
    fn explicit_compiler_override_is_preserved() {
        for compiler in [Dx12Compiler::Fxc, Dx12Compiler::Auto, Dx12Compiler::DynamicDxc { dxc_path: "custom/dxcompiler.dll".into() }] {
            let mut options = NativeOptions::default();
            configure(&mut options, None, Some(compiler.clone()));
            let WgpuSetup::CreateNew(create) = options.wgpu_options.wgpu_setup else {
                panic!("default native options must create a wgpu instance");
            };
            match (compiler, create.instance_descriptor.backend_options.dx12.shader_compiler) {
                (Dx12Compiler::Fxc, Dx12Compiler::Fxc) | (Dx12Compiler::Auto, Dx12Compiler::Auto) => {}
                (Dx12Compiler::DynamicDxc { dxc_path: expected }, Dx12Compiler::DynamicDxc { dxc_path: actual }) => {
                    assert_eq!(actual, expected);
                }
                (expected, actual) => panic!("compiler override changed from {expected:?} to {actual:?}"),
            }
        }
    }

    #[test]
    fn explicit_backend_override_is_preserved() {
        for backends in [Backends::VULKAN, Backends::GL, Backends::DX12, Backends::VULKAN | Backends::DX12, Backends::empty()] {
            assert_eq!(configured_backends(Some(backends)), backends);
        }
    }
}

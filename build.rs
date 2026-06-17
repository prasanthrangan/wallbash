use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let compiler = shaderc::Compiler::new().unwrap();
    let mut options = shaderc::CompileOptions::new().unwrap();
    options.set_optimization_level(shaderc::OptimizationLevel::Performance);

    let shader_dir = Path::new("shaders");
    for entry in fs::read_dir(shader_dir).expect("Failed to read shaders directory") {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("comp") {
            let name = path.file_name().unwrap().to_str().unwrap();
            compile_shader(&compiler, &options, &out_dir, name);
        }
    }
}

fn compile_shader(
    compiler: &shaderc::Compiler,
    options: &shaderc::CompileOptions,
    out_dir: &str,
    name: &str,
) {
    let path = Path::new("shaders").join(name);
    let source = fs::read_to_string(&path).expect("Failed to read shader");
    let result = compiler
        .compile_into_spirv(&source, shaderc::ShaderKind::Compute, name, "main", Some(options))
        .expect("Failed to compile shader");
    let out_name = Path::new(out_dir).join(format!("{}.spv", name));
    fs::write(out_name, result.as_binary_u8()).expect("Failed to write SPIR-V");
}


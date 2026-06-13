use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let path = Path::new("shaders").join("blur.comp");
    let source = fs::read_to_string(&path).expect("Failed to read blur.comp");
    let compiler = shaderc::Compiler::new().unwrap();
    let mut options = shaderc::CompileOptions::new().unwrap();
    options.set_optimization_level(shaderc::OptimizationLevel::Performance);
    let result = compiler
        .compile_into_spirv(&source, shaderc::ShaderKind::Compute, "blur.comp", "main", Some(&options))
        .unwrap();
    let out_name = Path::new(&out_dir).join("blur.comp.spv");
    fs::write(out_name, result.as_binary_u8()).unwrap();
}


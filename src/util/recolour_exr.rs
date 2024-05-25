use exr::prelude::*;
use rayon::prelude::*;
use config::Config;

use colorgrad::{Color, CustomGradient, Interpolation, BlendMode};

use std::fs;
use std::time::Instant;

use crate::util::generate_default_palette;

pub struct RecolourExr {
    palette_buffer: Vec<Color>,
    files: Vec<String>,
    palette_iteration_span: f32,
    palette_offset: f32
}

impl RecolourExr {
    pub fn new(settings: Config) -> Self {
        let (_, palette_buffer) = if let Ok(colour_values) = settings.get_array("palette") {
            let mut colors = colour_values.chunks_exact(3).map(|value| {
                Color::from_rgba8(value[0].clone().into_int().unwrap() as u8, 
                    value[1].clone().into_int().unwrap() as u8, 
                    value[2].clone().into_int().unwrap() as u8,
                    255)
            }).collect::<Vec<Color>>();

            if colors[0] != *colors.last().unwrap() {
                colors.push(colors[0].clone());
            };

            let mut number_colors = colors.len();

            if settings.get_bool("palette_cyclic").unwrap_or(true) {
                number_colors -= 1;
            }

            let palette_generator = CustomGradient::new()
                .colors(&colors[0..number_colors])
                .interpolation(Interpolation::CatmullRom)
                .mode(BlendMode::Oklab)
                .build().unwrap();

            (colors, palette_generator.colors(number_colors * 64))
        } else {
            generate_default_palette()
        };

        let palette_iteration_span = settings.get_float("palette_iteration_span").unwrap_or(10.0) as f32;
        let palette_offset = settings.get_float("iteration_offset").unwrap_or(0.0) as f32;

        let paths = fs::read_dir("./output/").unwrap();
        let mut exr_files = Vec::new();
    
        for path in paths {
            let name = path.unwrap()
                .path()
                .to_owned()
                .to_str()
                .unwrap()
                .to_string();
            
            if name.contains(".exr") {
                exr_files.push(name)
            }
        };

        RecolourExr {
            palette_buffer,
            files: exr_files,
            palette_iteration_span,
            palette_offset
        }
    }

    pub fn colour(&self) {
        let colouring_time = Instant::now();

        (&self.files).into_par_iter()
        .for_each(|exr_file| {
            let raw_data = read_all_data_from_file(&exr_file).unwrap();

            let mut iterations = Vec::new();
            let mut smooth = Vec::new();

            for layer in &raw_data.layer_data {
                for channel in &layer.channel_data.list {
                    match &channel.sample_data{
                    Levels::Singular(samples) => {
                        match samples {
                            FlatSamples::F16(f16_vec) => {
                                smooth = f16_vec.clone();
                            },
                            FlatSamples::F32(_) => {},
                            FlatSamples::U32(u32_vec) => {
                                iterations = u32_vec.clone();
                            }
                        };
                    },
                    Levels::Mip { level_data, .. } => {
                        // Handle Mip levels if necessary
                        println!("Mip levels: {:?}", level_data);
                    },
                    Levels::Rip { level_data, .. } => {
                        // Handle Rip levels if necessary
                        println!("Rip levels: {:?}", level_data);
                    },
                }
                }
            }

            let file_name = exr_file.split(".exr").collect::<Vec<_>>()[0];
            let dimensions = raw_data.attributes.display_window.size;

            println!("{}.exr", file_name);

            let mut rgb_buffer = vec![0u8; iterations.len() * 3];
            
            for i in 0..iterations.len() {
                if iterations[i] == 0xFFFFFFFF {
                    rgb_buffer[3 * i] = 0u8;
                    rgb_buffer[3 * i + 1] = 0u8;
                    rgb_buffer[3 * i + 2] = 0u8;
                } else {
                    let temp = self.palette_buffer.len() as f32 * ((iterations[i] as f32 + smooth[i].to_f32()) / self.palette_iteration_span + self.palette_offset).fract();

                    let pos1 = temp.floor() as usize;
                    let pos2 = if pos1 == (self.palette_buffer.len() - 1) {
                        0
                    } else {
                        pos1 + 1
                    };

                    let frac = temp.fract() as f64;

                    let (r, g, b, _) = self.palette_buffer[pos1].interpolate_rgb(&self.palette_buffer[pos2], frac).to_linear_rgba_u8();

                    rgb_buffer[3 * i] = r; 
                    rgb_buffer[3 * i + 1] = g; 
                    rgb_buffer[3 * i + 2] = b;
                }
            }

            image::save_buffer(file_name.to_owned() + ".png", &rgb_buffer, dimensions.x() as u32, dimensions.y() as u32, image::ColorType::Rgb8).unwrap();
        });

        println!("Recolouring {} images took {} ms.", self.files.len(), colouring_time.elapsed().as_millis());
    }
}

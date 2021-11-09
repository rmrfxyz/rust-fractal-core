use crate::util::{ComplexArbitrary, ComplexExtended, ComplexFixed, FloatExtended, FractalType, PixelData, ProgressCounters, data_export::*, extended_to_string_long, extended_to_string_short, generate_default_palette, generate_pascal_coefficients, get_approximation_terms, get_data_coloring_type_from_settings, get_delta_top_left, get_fractal_type_from_settings, string_to_extended};
use crate::math::{SeriesApproximation, Perturbation, Reference, BoxPeriod};

use std::{sync::{atomic::AtomicBool}, time::{Duration, Instant}};
use std::io::Write;
use std::cmp::{min, max};

// use rand::seq::SliceRandom;
use rand_distr::Distribution;

use colorgrad::{Color, CustomGradient, Interpolation, BlendMode};

use rayon::prelude::*;
use config::Config;

use std::thread;
use std::sync::{Arc, mpsc};
use std::sync::atomic::{Ordering, AtomicUsize};
use std::collections::HashMap;

use parking_lot::Mutex;

const FRACTAL_TYPE: usize = 0;
const FRACTAL_POWER: usize = 2;

pub struct FractalRenderer {
    pub image_width: usize,
    pub image_height: usize,
    pub total_pixels: usize,
    pub rotate: f64,
    pub zoom: FloatExtended,
    pub auto_adjust_iterations: bool,
    pub maximum_iteration: usize,
    pub glitch_percentage: f64,
    pub data_export: Arc<Mutex<DataExport>>,
    pub remaining_frames: usize,
    frame_offset: usize,
    pub zoom_scale_factor: f64,
    pub center_reference: Reference,
    pub series_approximation: SeriesApproximation,
    pub period_finding: BoxPeriod,
    render_indices: Vec<usize>,
    pub remove_centre: bool,
    pub data_type: DataType,
    pub jitter: bool,
    pub jitter_factor: f64,
    show_output: bool,
    pub progress: ProgressCounters,
    pub render_time: u128,
    pub fractal_type: FractalType,
    pub root_zoom_factor: f64,
    pub pascal: Vec<f64>
}

impl FractalRenderer {
    pub fn new(settings: Config) -> Self {
        let image_width = settings.get_int("image_width").unwrap_or(1000) as usize;
        let image_height = settings.get_int("image_height").unwrap_or(1000) as usize;

        let rotate = settings.get_float("rotate").unwrap_or(0.0).to_radians();
        let maximum_iteration = settings.get_int("iterations").unwrap_or(1000) as usize;

        let initial_zoom = settings.get_str("zoom").unwrap_or_else(|_| String::from("1E0")).to_ascii_uppercase();
        let center_real = settings.get_str("real").unwrap_or_else(|_| String::from("-0.75"));
        let center_imag = settings.get_str("imag").unwrap_or_else(|_| String::from("0.0"));

        let approximation_order = settings.get_int("approximation_order").unwrap_or(0) as usize;
        let glitch_percentage = settings.get_float("glitch_percentage").unwrap_or(0.001);
        let remaining_frames = settings.get_int("frames").unwrap_or(1) as usize;
        let frame_offset = settings.get_int("frame_offset").unwrap_or(0) as usize;
        let zoom_scale_factor = settings.get_float("zoom_scale").unwrap_or(2.0);
        let display_glitches = settings.get_bool("display_glitches").unwrap_or(false);

        let auto_adjust_iterations = settings.get_bool("auto_adjust_iterations").unwrap_or(true);
        let series_approximation_tiled = settings.get_bool("series_approximation_tiled").unwrap_or(true);
        let series_approximation_enabled = settings.get_bool("series_approximation_enabled").unwrap_or(true);

        let probe_sampling = settings.get_int("probe_sampling").unwrap_or(3) as usize;
        let remove_centre = settings.get_bool("remove_centre").unwrap_or(false);

        let palette_iteration_span = settings.get_float("palette_iteration_span").unwrap_or(100.0) as f32;
        let palette_offset = settings.get_float("palette_offset").unwrap_or(0.0) as f32;
        let palette_cyclic = settings.get_bool("palette_cyclic").unwrap_or(true);

        let distance_color = settings.get_bool("distance_color").unwrap_or(false);

        let lighting = settings.get_bool("lighting").unwrap_or(true);

        let lighting_direction = settings.get_float("lighting_direction").unwrap_or(30.0) as f32;
        let lighting_azimuth = settings.get_float("lighting_azimuth").unwrap_or(35.0) as f32;
        let lighting_opacity = settings.get_float("lighting_opacity").unwrap_or(0.75) as f32;
        let lighting_ambient = settings.get_float("lighting_ambient").unwrap_or(0.4) as f32;
        let lighting_diffuse = settings.get_float("lighting_diffuse").unwrap_or(0.5) as f32;
        let lighting_specular = settings.get_float("lighting_specular").unwrap_or(0.5) as f32;
        let lighting_shininess = settings.get_int("lighting_shininess").unwrap_or(20) as i32;

        let distance_transition = settings.get_float("distance_transition").unwrap_or(0.0) as f32;

        let valid_iteration_probe_multiplier = settings.get_float("valid_iteration_probe_multiplier").unwrap_or(0.02) as f32;
        let glitch_tolerance = settings.get_float("glitch_tolerance").unwrap_or(1.4e-6) as f64;
        let data_storage_interval = settings.get_int("data_storage_interval").unwrap_or(10) as usize;

        let fractal_type = get_fractal_type_from_settings(&settings);

        let (coloring_type, data_type) = get_data_coloring_type_from_settings(&settings);

        let stripe_scale = settings.get_float("stripe_scale").unwrap_or(1.0) as f32;

        let jitter = settings.get_bool("jitter").unwrap_or(false);
        let jitter_factor = settings.get_float("jitter_factor").unwrap_or(0.2);
        let show_output = settings.get_bool("show_output").unwrap_or(true);
        
        let export_type = match settings.get_str("export").unwrap_or_else(|_| String::from("COLOUR")).to_ascii_uppercase().as_ref() {
            "GUI" => ExportType::Gui,
            "RAW" | "EXR" => ExportType::Raw,
            "BOTH" => ExportType::Both,
            _ => ExportType::Color
        };

        let (palette_buffer, palette_interpolated_buffer) = if let Ok(colour_values) = settings.get_array("palette") {
            let mut colors = colour_values.chunks_exact(3).map(|value| {
                Color::from_rgb_u8(value[0].clone().into_int().unwrap() as u8, 
                    value[1].clone().into_int().unwrap() as u8, 
                    value[2].clone().into_int().unwrap() as u8)
            }).collect::<Vec<Color>>();

            if colors[0] != *colors.last().unwrap() {
                colors.push(colors[0].clone());
            };

            let mut number_colors = colors.len();

            if palette_cyclic {
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

        let mut zoom = string_to_extended(&initial_zoom);
        let delta_pixel =  (-2.0 * (4.0 / image_height as f64 - 2.0) / zoom) / image_height as f64;
        let radius = delta_pixel * image_width as f64;
        let precision = max(64, -radius.exponent + 64);

        let center_location = ComplexArbitrary::with_val(
            precision as u32,
            ComplexArbitrary::parse("(".to_owned() + &center_real + "," + &center_imag + ")").expect("provided location not valid"));

        let zero = ComplexArbitrary::with_val(
            center_location.prec().0 as u32,
            ComplexArbitrary::parse("(0.0,0.0)").expect("provided location not valid"));

        let auto_approximation = get_approximation_terms(approximation_order, image_width, image_height);

        let reference = Reference::new(zero, 
            center_location, 
            0, 
            maximum_iteration, 
            data_storage_interval,
            glitch_tolerance,
            zoom);

        let series_approximation = SeriesApproximation::new_central(auto_approximation, 
            maximum_iteration, 
            FloatExtended::new(0.0, 0), 
            probe_sampling,
            series_approximation_tiled,
            series_approximation_enabled,
            valid_iteration_probe_multiplier,
            data_storage_interval);

        let temporary_delta = ComplexExtended::new2(0.0, 0.0, 0);

        let period_finding = BoxPeriod::new(temporary_delta, [temporary_delta, temporary_delta, temporary_delta, temporary_delta]);

        let render_indices = FractalRenderer::generate_render_indices(image_width, image_height, remove_centre, zoom_scale_factor, export_type);

        // Change the zoom level to the correct one for the frame offset
        for _ in 0..frame_offset {
            zoom.mantissa /= zoom_scale_factor;
            zoom.reduce();
        }

        let data_export = Arc::new(Mutex::new(
            DataExport::new(
                    image_width, 
                    image_height, 
                    display_glitches, 
                    palette_buffer, 
                    palette_interpolated_buffer, 
                    palette_cyclic, 
                    palette_iteration_span, 
                    palette_offset, 
                    distance_transition, 
                    stripe_scale,
                    distance_color,
                    lighting,
                    coloring_type, 
                    data_type, 
                    fractal_type, 
                    export_type)
        ));

        data_export.lock().change_lighting(lighting_direction, lighting_azimuth, lighting_opacity, lighting_ambient, lighting_diffuse, lighting_specular, lighting_shininess);

        FractalRenderer {
            image_width,
            image_height,
            total_pixels: render_indices.len(),
            rotate,
            zoom,
            auto_adjust_iterations,
            maximum_iteration,
            glitch_percentage,
            data_export,
            remaining_frames,
            frame_offset,
            zoom_scale_factor,
            center_reference: reference,
            series_approximation,
            period_finding,
            render_indices,
            remove_centre,
            data_type,
            jitter,
            jitter_factor,
            show_output,
            progress: ProgressCounters::new(maximum_iteration),
            render_time: 0,
            fractal_type,
            root_zoom_factor: 0.0,
            pascal: generate_pascal_coefficients(FRACTAL_POWER + 1)
        }
    }

    pub fn render_frame(&mut self, frame_index: usize, filename: String, stop_flag: Arc<AtomicBool>) {
        self.progress.reset();
        
        if self.show_output {
            print!(" {:<15}", extended_to_string_short(self.zoom));
            std::io::stdout().flush().unwrap();
        };

        let frame_time = Instant::now();
        let approximation_time = Instant::now();

        let (tx, rx) = mpsc::channel();

        if self.show_output {
            let thread_counter_1 = self.progress.reference.clone();
            let thread_counter_2 = self.progress.series_approximation.clone();
            let thread_counter_3 = self.progress.reference_maximum.clone();
            let thread_counter_4 = self.progress.series_validation.clone();
            print!("|               ");

            thread::spawn(move || {
                loop {
                    match rx.try_recv() {
                        Ok(_) => {
                            break;
                        },
                        Err(_) => {
                            // 40% weighting to first reference, 40% to SA calculation, 20% to SA checking
                            let mut percentage_complete = 0.0;

                            // println!("{} {} {}", thread_counter_1.get(), thread_counter_2.get(), thread_counter_3.get());

                            percentage_complete += 45.0 * thread_counter_1.load(Ordering::Relaxed) as f64 / thread_counter_3.load(Ordering::Relaxed) as f64;
                            percentage_complete += 45.0 * thread_counter_2.load(Ordering::Relaxed) as f64 / thread_counter_3.load(Ordering::Relaxed) as f64;
                            percentage_complete += 10.0 * thread_counter_4.load(Ordering::Relaxed) as f64 / 2.0;

                            print!("\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08{:^14}", format!("{:.2}%", percentage_complete));
                            std::io::stdout().flush().unwrap();
                        }
                    };

                    thread::sleep(Duration::from_millis(100));
                };
            });
        };

        if frame_index == 0 {
            self.data_export.lock().maximum_iteration = self.maximum_iteration;

            self.center_reference.run::<FRACTAL_TYPE, FRACTAL_POWER>(&self.progress.reference, &self.progress.reference_maximum, &stop_flag);

            if self.stop_rendering(&stop_flag, frame_time) {
                tx.send(()).unwrap();
                return;
            };
            
            self.series_approximation.maximum_iteration = self.center_reference.current_iteration;
            self.series_approximation.generate_approximation(&self.center_reference, &self.progress.series_approximation, &stop_flag);
        } else {
            let mut export = self.data_export.lock();

            // If the image width/height changes intraframe (GUI) we need to regenerate some things
            if export.image_width != self.image_width || export.image_height != self.image_height {
                self.render_indices = FractalRenderer::generate_render_indices(self.image_width, self.image_height, self.remove_centre, self.zoom_scale_factor, export.export_type);

                export.centre_removed = self.remove_centre;
                export.image_width = self.image_width;
                export.image_height = self.image_height;
            }
            
            export.clear_buffers();

            drop(export);

            // Check to see if the series approximation order has changed intraframe
            if self.series_approximation.enabled && self.series_approximation.order != self.series_approximation.generated_order {
                self.series_approximation.min_valid_iteration = 1;
                self.series_approximation.generate_approximation(&self.center_reference, &self.progress.series_approximation, &stop_flag);
            }
        }

        self.data_type = match self.data_export.lock().coloring_type {
            ColoringType::SmoothIteration | ColoringType::StepIteration => DataType::Iteration,
            ColoringType::Stripe => DataType::Stripe,
            ColoringType::DistanceStripe => DataType::DistanceStripe,
            _ => DataType::Distance
        };

        if self.stop_rendering(&stop_flag, frame_time) {
            tx.send(()).unwrap();
            return;
        };
        
        let cos_rotate = self.rotate.cos();
        let sin_rotate = self.rotate.sin();

        let delta_pixel = 4.0 / ((self.image_height - 1) as f64 * self.zoom.mantissa);

        let delta_pixel_cos = delta_pixel * cos_rotate;
        let delta_pixel_sin = delta_pixel * sin_rotate;

        let delta_top_left = get_delta_top_left(delta_pixel, self.image_width, self.image_height, cos_rotate, sin_rotate);
        let delta_pixel_extended = FloatExtended::new(delta_pixel, -self.zoom.exponent);

        let minimum_dimension = min(self.image_width, self.image_height);

        self.series_approximation.delta_pixel_square = if minimum_dimension < 1000 {
            let fixed_delta_pixel_extended = FloatExtended::new(4.0 / (999.0 * self.zoom.mantissa), -self.zoom.exponent);
            
            fixed_delta_pixel_extended * fixed_delta_pixel_extended
        } else {
            delta_pixel_extended * delta_pixel_extended
        };

        // Used for placing the probe points
        self.series_approximation.check_approximation(
            delta_top_left, 
            -self.zoom.exponent, 
            cos_rotate, 
            sin_rotate, 
            delta_pixel, 
            self.image_width,
            self.image_height,
            &self.center_reference,
            &self.progress.series_validation);

        self.progress.min_series_approximation.store(self.series_approximation.min_valid_iteration, Ordering::SeqCst);
        self.progress.max_series_approximation.store(self.series_approximation.max_valid_iteration, Ordering::SeqCst);

        tx.send(()).unwrap();

        if self.stop_rendering(&stop_flag, frame_time) {
            return;
        };

        if self.show_output {
            print!("\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08{:<15}", approximation_time.elapsed().as_millis());
            print!("| {:<15}", self.series_approximation.min_valid_iteration);
            print!("| {:<6}", self.series_approximation.order);
            print!("| {:<15}", self.maximum_iteration);
            std::io::stdout().flush().unwrap();
        };

        if self.remove_centre != self.data_export.lock().centre_removed {
            self.render_indices = FractalRenderer::generate_render_indices(self.image_width, self.image_height, self.remove_centre, self.zoom_scale_factor, self.data_export.lock().export_type);
            self.data_export.lock().centre_removed = self.remove_centre;
        }

        let jacobian_default = [ComplexExtended::new2(1.0, 0.0, 0), ComplexExtended::new2(0.0, 1.0, 0)];
        let sampling_resolution_width = (self.series_approximation.probe_sampling - 1) as f64 / self.image_width as f64;
        let sampling_resolution_height = (self.series_approximation.probe_sampling - 1) as f64 / self.image_height as f64;

        let normal = rand_distr::Normal::new(0.0, self.jitter_factor).unwrap();

        let mut pixel_data = (&self.render_indices).into_par_iter()
            .map(|index| {
                let mut i = (index % self.image_width) as f64;
                let mut j = (index / self.image_width) as f64;

                let chosen_iteration = if self.series_approximation.enabled {
                    if self.series_approximation.tiled {
                        let test1 = (i * sampling_resolution_width).floor() as usize;
                        let test2 = (j * sampling_resolution_height).floor() as usize;

                        let index = test2 * (self.series_approximation.probe_sampling - 1) + test1;

                        self.series_approximation.valid_interpolation[index]
                    } else {
                        self.series_approximation.min_valid_iteration
                    }
                } else {
                    1
                };

                if self.jitter {
                    let mut rng = rand::thread_rng();

                    i += normal.sample(&mut rng);
                    j += normal.sample(&mut rng);
                }

                let element = ComplexFixed::new(
                    i * delta_pixel_cos - j * delta_pixel_sin + delta_top_left.re, 
                    i * delta_pixel_sin + j * delta_pixel_cos + delta_top_left.im
                );

                let point_delta = ComplexExtended::new(element, -self.zoom.exponent);

                PixelData {
                    index: *index,
                    iteration: chosen_iteration,
                    delta_reference: point_delta,
                    delta_current: point_delta,
                    jacobian_current: jacobian_default,
                    glitched: false,
                    z_norm: 0.0,
                    stripe_storage: [ComplexFixed::new(0.0, 0.0); 4],
                    stripe_iteration: 0,
                }
            }).collect::<Vec<PixelData>>();

        if self.stop_rendering(&stop_flag, frame_time) {
            return;
        };
        
        let iteration_time = Instant::now();

        let total_pixels = self.render_indices.len() as f64;

        let (tx, rx) = mpsc::channel();

        if self.show_output {
            let thread_counter = Arc::clone(&self.progress.iteration);
            print!("|               ");

            thread::spawn(move || {
                loop {
                    thread::sleep(Duration::from_millis(100));
                    match rx.try_recv() {
                        Ok(_) => {
                            break;
                        },
                        Err(_) => {
                            print!("\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08{:^14}", format!("{:.2}%", 100.0 * thread_counter.load(Ordering::SeqCst) as f64 / total_pixels));
                            std::io::stdout().flush().unwrap();
                        }
                    };
                };
            });
        };

        let values = [16usize, 8, 4, 2, 1];
        let mut previous_value = 0;
        let number_pixels = self.render_indices.len();

        for &value in values.iter() {
            let end_value = number_pixels / (value * value);
            let chunk_size = max((end_value - previous_value) / 512, 8);

            match self.data_type {
                DataType::Distance => {
                    Perturbation::iterate::<1, FRACTAL_TYPE, FRACTAL_POWER>(&mut pixel_data[previous_value..end_value], &self.center_reference, &self.progress.iteration, &stop_flag, self.data_export.clone(), delta_pixel_extended, value, chunk_size, &self.series_approximation, true, &self.pascal);
                },
                DataType::Stripe => {
                    Perturbation::iterate::<2, FRACTAL_TYPE, FRACTAL_POWER>(&mut pixel_data[previous_value..end_value], &self.center_reference, &self.progress.iteration, &stop_flag, self.data_export.clone(), delta_pixel_extended, value, chunk_size, &self.series_approximation, true, &self.pascal);
                },
                DataType::DistanceStripe => {
                    Perturbation::iterate::<3, FRACTAL_TYPE, FRACTAL_POWER>(&mut pixel_data[previous_value..end_value], &self.center_reference, &self.progress.iteration, &stop_flag, self.data_export.clone(), delta_pixel_extended, value, chunk_size, &self.series_approximation, true, &self.pascal);
                },
                _ => {
                    Perturbation::iterate::<0, FRACTAL_TYPE, FRACTAL_POWER>(&mut pixel_data[previous_value..end_value], &self.center_reference, &self.progress.iteration, &stop_flag, self.data_export.clone(), delta_pixel_extended, value, chunk_size, &self.series_approximation, true, &self.pascal);
                }
            }

            previous_value = end_value;
        }

        tx.send(()).unwrap();

        if self.stop_rendering(&stop_flag, frame_time) {
            return;
        };

        if self.show_output {
            print!("\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08{:<15}", iteration_time.elapsed().as_millis());
            std::io::stdout().flush().unwrap();
        };
        
        // let correction_time = Instant::now();

        // // Remove all non-glitched points from the remaining points
        // pixel_data.retain(|packet| {
        //     packet.glitched
        // });

        // let glitched_pixels = pixel_data.len() as f64;
        // self.progress.glitched_maximum.fetch_add(pixel_data.len(), Ordering::SeqCst);

        // let complete_pixels = total_pixels - glitched_pixels;

        // let (tx, rx) = mpsc::channel();

        // if self.show_output {
        //     let thread_counter = Arc::clone(&self.progress.iteration);
        //     print!("|               ");

        //     thread::spawn(move || {
        //         loop {
        //             thread::sleep(Duration::from_millis(100));
        //             match rx.try_recv() {
        //                 Ok(_) => {
        //                     break;
        //                 },
        //                 Err(_) => {
        //                     print!("\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08{:^14}", format!("{:.2}%", 100.0 * (thread_counter.load(Ordering::Relaxed) as f64 - complete_pixels) / glitched_pixels));
        //                     std::io::stdout().flush().unwrap();
        //                 }
        //             };
        //         };
        //     });
        // };

        // // Goes through all glitches and solved them - no need for glitch percentage at this time
        // if pixel_data.len() > 0 {
        //     self.resolve_glitches(&mut pixel_data, &stop_flag, frame_time, delta_pixel_extended, None);
        // }

        // tx.send(()).unwrap();

        // if self.show_output {
        //     print!("\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08\x08{:<15}", correction_time.elapsed().as_millis());
        //     print!("| {:<6}", self.progress.reference_count.load(Ordering::SeqCst));
        //     std::io::stdout().flush().unwrap();
        // };
        
        self.data_export.lock().save(&filename, self.series_approximation.order, &extended_to_string_long(self.zoom));

        self.render_time = frame_time.elapsed().as_millis();

        if self.show_output {
            println!("| {:<15}", frame_time.elapsed().as_millis());
            std::io::stdout().flush().unwrap();
        }
    }

    // Recursive glitch solving by glitch levels
    // Start with a central reference that has ALL data stored for each iteration past the min skip
    pub fn resolve_glitches(&self, pixel_data: &mut [PixelData], stop_flag: &Arc<AtomicBool>, frame_time: Instant, delta_pixel_extended: FloatExtended, previous_reference: Option<Reference>) {
        let mut iteration_map: HashMap<usize, Vec<PixelData>> = HashMap::new();

        // Sort into bins to process
        pixel_data.iter()
            .for_each(|pixel| {
                match iteration_map.get_mut(&pixel.iteration) {
                    Some(pixels) => {
                        pixels.push(pixel.clone());
                    }
                    None => {
                        iteration_map.insert(pixel.iteration, vec![pixel.clone()]);
                    }
                }
            });

        let previous_reference = match previous_reference {
            Some(reference) => {
                reference
            },
            None => {
                let mut lowest_iteration = usize::MAX;
                let mut highest_iteration = 0;
        
                iteration_map.iter().for_each(|(&index, _)| {
                    if index < lowest_iteration {
                        lowest_iteration = index;
                    }

                    if index > highest_iteration {
                        highest_iteration = index;
                    }
                });

                let mut previous_reference = self.center_reference.get_central_glitch_resolving_reference(lowest_iteration);

                // Modify the precision of the reference to the current image scale
                let radius = delta_pixel_extended * self.image_width as f64;
                let precision = max(64, -radius.exponent + 64) as u32;

                
                previous_reference.c.set_prec(precision);
                previous_reference.z.set_prec(precision);

                let previous_maximum_iteration = previous_reference.maximum_iteration;

                // We cap this to avoid running the reference too long when the central point has a large amount of iterations relative to the rest of the pixels
                previous_reference.maximum_iteration = min(highest_iteration + 10000, previous_maximum_iteration);

                previous_reference.run::<FRACTAL_TYPE, FRACTAL_POWER>(&Arc::new(AtomicUsize::new(0)), &Arc::new(AtomicUsize::new(0)), &stop_flag);

                // Reset the maximum iteration of the reference back to normal to make it seem the reference escaped early
                previous_reference.maximum_iteration = previous_maximum_iteration;

                previous_reference
            }
        };

        if stop_flag.load(Ordering::SeqCst) {
            return;
        };

        iteration_map.par_iter_mut()
            .for_each(|(iteration, pixel_data)| {
                let glitch_reference_pixel = pixel_data.iter().min_by(|i, j| {
                    i.z_norm.partial_cmp(&j.z_norm).unwrap()
                }).unwrap().clone();

                let mut glitch_reference = previous_reference.get_glitch_resolving_reference(*iteration, glitch_reference_pixel.delta_reference, glitch_reference_pixel.delta_current);
                glitch_reference.run::<FRACTAL_TYPE, FRACTAL_POWER>(&Arc::new(AtomicUsize::new(0)), &Arc::new(AtomicUsize::new(0)), &stop_flag);

                self.progress.reference_count.fetch_add(1, Ordering::SeqCst);
    
                if stop_flag.load(Ordering::SeqCst) {
                    return;
                };

                pixel_data.par_iter_mut()
                    .for_each(|pixel| {
                        pixel.glitched = false;
                        pixel.delta_current -= glitch_reference_pixel.delta_current;
                        pixel.delta_reference -= glitch_reference_pixel.delta_reference;
                });

                let chunk_size = max(pixel_data.len() / 512, 4);


                match self.data_type {
                    DataType::Distance => {
                        Perturbation::iterate::<1, FRACTAL_TYPE, FRACTAL_POWER>(pixel_data, &glitch_reference, &self.progress.iteration, &stop_flag, self.data_export.clone(), delta_pixel_extended, 1, chunk_size, &self.series_approximation, false, &self.pascal);

                    },
                    DataType::Stripe => {
                        Perturbation::iterate::<2, FRACTAL_TYPE, FRACTAL_POWER>(pixel_data, &glitch_reference, &self.progress.iteration, &stop_flag, self.data_export.clone(), delta_pixel_extended, 1, chunk_size, &self.series_approximation, false, &self.pascal);

                    },
                    DataType::DistanceStripe => {
                        Perturbation::iterate::<3, FRACTAL_TYPE, FRACTAL_POWER>(pixel_data, &glitch_reference, &self.progress.iteration, &stop_flag, self.data_export.clone(), delta_pixel_extended, 1, chunk_size, &self.series_approximation, false, &self.pascal);

                    },
                    _ => {
                        Perturbation::iterate::<0, FRACTAL_TYPE, FRACTAL_POWER>(pixel_data, &glitch_reference, &self.progress.iteration, &stop_flag, self.data_export.clone(), delta_pixel_extended, 1, chunk_size, &self.series_approximation, false, &self.pascal);
                    }
                }

                pixel_data.retain(|packet| {
                    packet.glitched
                });

                if pixel_data.len() > 0 {
                    self.resolve_glitches(pixel_data, stop_flag, frame_time, delta_pixel_extended, Some(glitch_reference))
                }
            });
    }

    pub fn stop_rendering(&mut self, stop_flag: &Arc<AtomicBool>, frame_time: Instant) -> bool {
        if stop_flag.load(Ordering::SeqCst) {
            self.render_time = frame_time.elapsed().as_millis();
            self.progress.reset();
            stop_flag.store(false, Ordering::SeqCst);

            println!();

            true
        } else {
            false
        }
    }

    // find the period given a box
    pub fn find_period(&mut self) {
        self.period_finding.find_period(&self.center_reference);
        // self.period_finding.find_atom_domain_period(&self.center_reference);
    }

    // Returns true if the maximum iterations has been increased
    pub fn adjust_iterations(&mut self) -> bool {
        if self.auto_adjust_iterations {
            if 4 * self.series_approximation.max_valid_iteration > self.maximum_iteration {
                self.maximum_iteration *= 3;
                self.maximum_iteration /= 2;
                return true;
            }

            if 8 * self.series_approximation.max_valid_iteration < self.maximum_iteration {
                self.maximum_iteration *= 3;
                self.maximum_iteration /= 4;

                if self.maximum_iteration < 1000 {
                    self.maximum_iteration = 1000;
                }

                self.data_export.lock().maximum_iteration = self.maximum_iteration;
                
                if self.center_reference.current_iteration > self.maximum_iteration {
                    self.center_reference.current_iteration = self.maximum_iteration;
                }

                self.center_reference.maximum_iteration = self.maximum_iteration;
            }
        }
        false
    }

    pub fn render(&mut self) {
        // Print out the status information
        if self.show_output {
            println!(" {:<15}| {:<15}| {:<15}| {:<6}| {:<15}| {:<15}| {:<15}| {:<6}| {:<15}", "Zoom", "Approx [ms]", "Skipped [it]", "Order", "Maximum [it]", "Iteration [ms]", "Correct [ms]", "Ref", "Frame [ms]");
        };

        let mut count = 0;

        while self.remaining_frames > 0 && self.zoom.to_float() > 0.5 {
            self.render_frame(count, 
                format!("output/{:08}_{}", count + self.frame_offset, 
                extended_to_string_short(self.zoom)), Arc::new(AtomicBool::new(false)));

            self.zoom.mantissa /= self.zoom_scale_factor;
            self.zoom.reduce();

            if self.zoom.to_float() < 1e10 {
                // Set these to start from the beginning
                self.series_approximation.valid_iteration_probe_multiplier = 1.0;

                // SA has some problems with precision with lots of terms at lot zoom levels
                if self.series_approximation.order > 8 {
                    // Overwrite the series approximation order
                    self.series_approximation.order = 8;
                    self.series_approximation.maximum_iteration = self.center_reference.current_iteration;
                    self.series_approximation.generate_approximation(&self.center_reference, &self.progress.series_approximation, &Arc::new(AtomicBool::new(false)));
                }

                // Logic in here to automatically adjust the maximum number of iterations
                // This is done arbitrarily and could be done in a config file if required
                if self.auto_adjust_iterations && self.maximum_iteration > 10000 {
                    let new_iteration_value = 10000;

                    if self.center_reference.current_iteration >= 10000 {
                        self.center_reference.current_iteration = new_iteration_value;
                    };

                    self.center_reference.maximum_iteration = new_iteration_value;
                    self.maximum_iteration = new_iteration_value;
                }
            } else if self.series_approximation.min_valid_iteration < 1000 && self.series_approximation.order > 16 {
                    self.series_approximation.order = 16;
                    self.series_approximation.generate_approximation(&self.center_reference, &self.progress.series_approximation, &Arc::new(AtomicBool::new(false)));
            } else if self.series_approximation.min_valid_iteration < 10000 && self.series_approximation.order > 32 {
                self.series_approximation.order = 32;
                self.series_approximation.generate_approximation(&self.center_reference, &self.progress.series_approximation, &Arc::new(AtomicBool::new(false)));
            }
            
            self.remaining_frames -= 1;
            count += 1;
        }
    }

    pub fn generate_render_indices(image_width: usize, image_height: usize, remove_centre: bool, zoom_scale_factor: f64, export_type: ExportType) -> Vec<usize> {
        // let time = Instant::now();

        let mut indices = Vec::with_capacity(image_width * image_height);

        let temp = 0.5 - 0.5 / zoom_scale_factor;
        let val1 = (image_width as f64 * temp).ceil() as usize;
        let val2 = (image_height as f64 * temp).ceil() as usize;

        match export_type {
            ExportType::Gui => {
                // Could order each subsection with circular ordering
                let values = [16, 8, 4, 2, 1];

                for (n, value) in values.iter().enumerate() {
                    for j in (0..image_height).step_by(*value) {
                        for i in (0..image_width).step_by(*value) {
                            if n == 0 || i & (values[n - 1] - 1) != 0 || j & (values[n - 1] - 1) != 0 {
                                if !remove_centre || (i <= val1 || i >= image_width - val1 || j <= val2 || j >= image_height - val2) {
                                    indices.push(j * image_width + i);
                                }

                            }
                        }
                    }
                }
            }
            _ => {
                for j in 0..image_height {
                    for i in 0..image_width {
                        if !remove_centre || (i <= val1 || i >= image_width - val1 || j <= val2 || j >= image_height - val2) {
                            indices.push(j * image_width + i);
                        }
                    }
                }
            }
        }

        // println!("generate indices took {}ms", time.elapsed().as_millis());

        indices
    }

    pub fn regenerate_from_settings(&mut self, settings: Config) {
        self.image_width = settings.get_int("image_width").unwrap_or(1000) as usize;
        self.image_height = settings.get_int("image_height").unwrap_or(1000) as usize;
        self.rotate = settings.get_float("rotate").unwrap_or(0.0).to_radians();
        self.maximum_iteration = settings.get_int("iterations").unwrap_or(1000) as usize;
        let initial_zoom = settings.get_str("zoom").unwrap_or_else(|_| String::from("1E0")).to_ascii_uppercase();
        let center_real = settings.get_str("real").unwrap_or_else(|_| String::from("-0.75"));
        let center_imag = settings.get_str("imag").unwrap_or_else(|_| String::from("0.0"));
        let approximation_order = settings.get_int("approximation_order").unwrap_or(0) as usize;
        self.glitch_percentage = settings.get_float("glitch_percentage").unwrap_or(0.001);
        self.remaining_frames = settings.get_int("frames").unwrap_or(1) as usize;
        self.frame_offset = settings.get_int("frame_offset").unwrap_or(0) as usize;
        self.zoom_scale_factor = settings.get_float("zoom_scale").unwrap_or(2.0);
        self.data_export.lock().display_glitches = settings.get_bool("display_glitches").unwrap_or(false);
        self.auto_adjust_iterations = settings.get_bool("auto_adjust_iterations").unwrap_or(true);

        let series_approximation_tiled = settings.get_bool("series_approximation_tiled").unwrap_or(true);
        let series_approximation_enabled = settings.get_bool("series_approximation_enabled").unwrap_or(true);

        let probe_sampling = settings.get_int("probe_sampling").unwrap_or(3) as usize;
        self.remove_centre = settings.get_bool("remove_centre").unwrap_or(true);

        self.data_export.lock().palette_iteration_span = settings.get_float("palette_iteration_span").unwrap_or(100.0) as f32;
        self.data_export.lock().palette_offset = settings.get_float("palette_offset").unwrap_or(0.0) as f32;
        self.data_export.lock().distance_transition = settings.get_float("distance_transition").unwrap_or(0.0) as f32;
        self.data_export.lock().distance_color = settings.get_bool("distance_color").unwrap_or(false);

        self.data_export.lock().lighting = settings.get_bool("lighting").unwrap_or(true);

        let lighting_direction = settings.get_float("lighting_direction").unwrap() as f32;
        let lighting_azimuth = settings.get_float("lighting_azimuth").unwrap() as f32;
        let lighting_opacity = settings.get_float("lighting_opacity").unwrap() as f32;
        let lighting_ambient = settings.get_float("lighting_ambient").unwrap() as f32;
        let lighting_diffuse = settings.get_float("lighting_diffuse").unwrap() as f32;
        let lighting_specular = settings.get_float("lighting_specular").unwrap() as f32;
        let lighting_shininess = settings.get_int("lighting_shininess").unwrap() as i32;

        self.data_export.lock().change_lighting(lighting_direction, lighting_azimuth, lighting_opacity, lighting_ambient, lighting_diffuse, lighting_specular, lighting_shininess);

        let valid_iteration_probe_multiplier = settings.get_float("valid_iteration_probe_multiplier").unwrap_or(0.02) as f32;
        let glitch_tolerance = settings.get_float("glitch_tolerance").unwrap_or(1.4e-6) as f64;
        let data_storage_interval = settings.get_int("data_storage_interval").unwrap_or(10) as usize;
        
        let coloring_type = match settings.get("coloring_type").unwrap_or_else(|_| String::from("smooth_iteration")).to_ascii_uppercase().as_ref() {
            "SMOOTH_ITERATION" | "SMOOTH" => ColoringType::SmoothIteration,
            "STEP_ITERATION" | "STEP" => ColoringType::StepIteration,
            "DISTANCE" => ColoringType::Distance,
            "STRIPE" => ColoringType::Stripe,
            "DISTANCE_STRIPE" => ColoringType::DistanceStripe,
            _ => ColoringType::SmoothIteration
        };

        let pixel_data_type = match coloring_type {
            ColoringType::SmoothIteration | ColoringType::StepIteration => DataType::Iteration,
            ColoringType::Stripe => DataType::Stripe,
            ColoringType::DistanceStripe => DataType::DistanceStripe,
            _ => DataType::Distance
        };

        self.data_export.lock().stripe_scale = settings.get_float("stripe_scale").unwrap_or(1.0) as f32;

        self.jitter = settings.get_bool("jitter").unwrap_or(false);
        self.jitter_factor = settings.get_float("jitter_factor").unwrap_or(0.2);
        self.show_output = settings.get_bool("show_output").unwrap_or(true);

        let mut zoom = string_to_extended(&initial_zoom);
        let delta_pixel =  (-2.0 * (4.0 / self.image_height as f64 - 2.0) / zoom) / self.image_height as f64;
        let radius = delta_pixel * self.image_width as f64;
        let precision = max(64, -radius.exponent + 64);

        let center_location = ComplexArbitrary::with_val(
            precision as u32,
            ComplexArbitrary::parse("(".to_owned() + &center_real + "," + &center_imag + ")").expect("provided location not valid"));
        let auto_approximation = get_approximation_terms(approximation_order, self.image_width, self.image_height);

        self.center_reference = Reference::new(center_location.clone(), 
            center_location, 
            1, 
            self.maximum_iteration, 
            data_storage_interval,
            glitch_tolerance,
            zoom);

        self.series_approximation = SeriesApproximation::new_central(auto_approximation, 
            self.maximum_iteration, 
            FloatExtended::new(0.0, 0), 
            probe_sampling,
            series_approximation_tiled,
            series_approximation_enabled,
            valid_iteration_probe_multiplier,
            data_storage_interval);

        let mut data_export = self.data_export.lock();

        if self.image_width != data_export.image_width || self.image_height != data_export.image_height {
            self.render_indices = FractalRenderer::generate_render_indices(self.image_width, self.image_height, self.remove_centre, self.zoom_scale_factor, self.data_export.lock().export_type);
            data_export.centre_removed = self.remove_centre;
        }

        self.total_pixels = self.render_indices.len();

        // Change the zoom level to the correct one for the frame offset
        for _ in 0..self.frame_offset {
            zoom.mantissa /= self.zoom_scale_factor;
            zoom.reduce();
        }

        self.zoom = zoom;

        self.progress.reset_all(self.maximum_iteration);

        data_export.image_width = self.image_width;
        data_export.image_height = self.image_height;
        data_export.data_type = pixel_data_type;
        data_export.coloring_type = coloring_type;

        data_export.clear_buffers();
    }
}
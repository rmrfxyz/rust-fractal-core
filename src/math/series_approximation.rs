use crate::util::{ComplexFixed, to_extended};
use crate::util::complex_extended::ComplexExtended;
use crate::math::reference::Reference;
use crate::util::float_extended::FloatExtended;
use rayon::prelude::*;

use std::cmp::max;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};

pub struct SeriesApproximation {
    pub maximum_iteration: usize,
    pub delta_pixel_square: FloatExtended,
    pub order: usize,
    pub generated_order: usize,
    pub coefficients: Vec<Vec<ComplexExtended>>,
    probe_start: Vec<ComplexExtended>,
    approximation_probes: Vec<Vec<ComplexExtended>>,
    approximation_probes_derivative: Vec<Vec<ComplexExtended>>,
    pub min_valid_iteration: usize,
    pub max_valid_iteration: usize,
    pub valid_iterations: Vec<usize>,
    pub valid_interpolation: Vec<usize>,
    pub probe_sampling: usize,
    pub tiled: bool,
    pub enabled: bool,
    pub valid_iteration_probe_multiplier: f32,
    pub data_storage_interval: usize,
}

impl SeriesApproximation {
    pub fn new_central(order: usize, 
        maximum_iteration: usize, 
        delta_pixel_square: FloatExtended, 
        probe_sampling: usize, 
        tiled: bool, 
        enabled: bool,
        valid_iteration_probe_multiplier: f32,
        data_storage_interval: usize) -> Self {

        // The current iteration is set to 1 as we set z = c
        SeriesApproximation {
            maximum_iteration,
            delta_pixel_square,
            order,
            generated_order: 0,
            coefficients: Vec::new(),
            probe_start: Vec::new(),
            approximation_probes: Vec::new(),
            approximation_probes_derivative: Vec::new(),
            min_valid_iteration: 1,
            max_valid_iteration: 1,
            valid_iterations: Vec::new(),
            valid_interpolation: Vec::new(),
            probe_sampling,
            tiled,
            enabled,
            valid_iteration_probe_multiplier,
            data_storage_interval
        }
    }

    pub fn generate_approximation(&mut self, center_reference: &Reference, series_approximation_counter: &Arc<AtomicUsize>, stop_flag: &Arc<AtomicBool>) {
        if !self.enabled {
            series_approximation_counter.store(1, Ordering::SeqCst);
            return;
        }

        series_approximation_counter.store(0, Ordering::SeqCst);

        // Reset the coefficients
        self.coefficients = vec![vec![ComplexExtended::new2(0.0, 0.0, 0); self.order as usize + 1]; 1];

        // 1th element is the z^2 + c, which is the 1st iteration
        self.coefficients[0][0] = to_extended(&center_reference.c);
        self.coefficients[0][1] = ComplexExtended::new2(1.0, 0.0, 0);

        let add_value = ComplexExtended::new2(1.0, 0.0, 0);

        let mut previous_coefficients = self.coefficients[0].clone();
        let mut next_coefficients = vec![ComplexExtended::new2(0.0, 0.0, 0); self.order as usize + 1];

        // Can be changed later into a better loop - this function could also return some more information
        // Go through all remaining iterations
        for i in 1..(self.maximum_iteration - 1) {
            if stop_flag.load(Ordering::SeqCst) {
                return
            };

            // This is checking if the approximation can step forward so takes the next iteration
            next_coefficients[0] = center_reference.reference_data_extended[i + 1];
            next_coefficients[1] = previous_coefficients[0] * previous_coefficients[1] * 2.0 + add_value;
            next_coefficients[0].reduce();
            next_coefficients[1].reduce();

            // Calculate the new coefficents
            for k in 2..=self.order {
                let mut sum = previous_coefficients[0] * previous_coefficients[k];

                for j in 1..=((k - 1) / 2) {
                    sum += previous_coefficients[j] * previous_coefficients[k - j];
                }
                sum *= 2.0;

                // If even, we include the mid term as well
                if k % 2 == 0 {
                    sum += previous_coefficients[k / 2] * previous_coefficients[k / 2];
                }

                sum.reduce();
                next_coefficients[k] = sum;
            }

            previous_coefficients = next_coefficients.clone();

            series_approximation_counter.fetch_add(1, Ordering::Relaxed);
            
            // only every 100th iteration (101, 201 etc)
            // This is 0, 100, 200 -> 1, 101, 201
            if i % self.data_storage_interval == 0 {
                self.coefficients.push(next_coefficients.clone());
            }

            // self.coefficients.push(next_coefficients);
        }

        // TODO maybe need something here to say series approximation was complete
        self.generated_order = self.order;
    }

    pub fn calculate_probes(&mut self, image_width: usize, image_height: usize, cos_rotate: f64, sin_rotate: f64, delta_top_left_mantissa: ComplexFixed<f64>, delta_top_left_exponent: i32, delta_pixel: f64) {
        // Delete the previous probes and calculate new ones
        self.probe_start = Vec::new();
        self.approximation_probes = Vec::new();
        self.approximation_probes_derivative = Vec::new();
        self.valid_interpolation = Vec::new();

        // Probes are stored in row first order
        for j in 0..self.probe_sampling {
            for i in 0..self.probe_sampling {

                let pos_x = image_width as f64 * (i as f64 / (self.probe_sampling as f64 - 1.0));
                let pos_y = image_height as f64 * (j as f64 / (self.probe_sampling as f64 - 1.0));

                // This could be changed to account for jittering if needed
                let real = pos_x * delta_pixel * cos_rotate - pos_y * delta_pixel * sin_rotate + delta_top_left_mantissa.re;
                let imag = pos_x * delta_pixel * sin_rotate + pos_y * delta_pixel * cos_rotate + delta_top_left_mantissa.im;

                self.add_probe(ComplexExtended::new2(
                    real, 
                    imag, delta_top_left_exponent));
            }
        }
    }

    pub fn iterate_probes(&mut self, center_reference: &Reference, valid_iterations: &mut [usize], selected_probes: Option<&[usize]>, test_val: usize, check_val: usize) {
        let mut current_probe_check_value = if self.min_valid_iteration > test_val {
            self.min_valid_iteration - test_val
        } else {
            1
        };

        let mut next_probe_check_value = if current_probe_check_value > test_val {
            current_probe_check_value - test_val
        } else {
            1
        };

        match selected_probes {
            Some(selected) => {
                selected.iter().for_each(|&index| valid_iterations[index] = current_probe_check_value);
            },
            None => {
                valid_iterations.iter_mut().for_each(|value| *value = current_probe_check_value);
            }
        };

        loop {
            valid_iterations.par_iter_mut().enumerate()
                .for_each(|(i, probe_iteration_level)| {
                    match selected_probes {
                        Some(selected) => {
                            if !selected.contains(&i) {
                                return;
                            }
                        },
                        None => {}
                    };

                    // check if the probe has already found its max skip
                    if *probe_iteration_level == current_probe_check_value {
                        let mut probe = self.evaluate(self.probe_start[i], *probe_iteration_level);

                        while *probe_iteration_level < self.maximum_iteration {
                            // step the probe points using perturbation
                            probe = probe * (center_reference.reference_data_extended[*probe_iteration_level] * 2.0 + probe);
                            probe += self.probe_start[i];

                            // This is not done on every iteration
                            if *probe_iteration_level % 250 == 0 {
                                probe.reduce();
                            }

                            // triggers on the first iteration when the next iteration is 1001, 1101 etc.
                            if *probe_iteration_level % self.data_storage_interval == 0 {
                                let series_probe = self.evaluate(self.probe_start[i], *probe_iteration_level + self.data_storage_interval);
                                let derivative_probe = self.evaluate_derivative(self.probe_start[i], *probe_iteration_level + self.data_storage_interval);

                                let relative_error = (probe - series_probe).norm_square();
                                let mut derivative = derivative_probe.norm_square();

                                // Check to make sure that the derivative is greater than or equal to 1
                                if derivative.to_float() < 1.0 {
                                    derivative.mantissa = 1.0;
                                    derivative.exponent = 0;
                                }

                                // The first element is reduced, the second might need to be reduced a little more
                                // Check that the error over the derivative is less than the pixel spacing
                                if relative_error / derivative > self.delta_pixel_square || relative_error.exponent > 0 {
                                    // TODO here need to investigate adaptively changing this to either data storage interval for 2 * test
                                    if *probe_iteration_level <= (current_probe_check_value + check_val + 1) && next_probe_check_value != 1 {
                                        *probe_iteration_level = next_probe_check_value;
                                        break;
                                    };

                                    *probe_iteration_level = if *probe_iteration_level > self.data_storage_interval {
                                        // Go back to the nearest value that was valid
                                        *probe_iteration_level - self.data_storage_interval + 1
                                    } else {
                                        1
                                    };

                                    break;
                                }
                            }

                            *probe_iteration_level += 1;
                        }

                        if *probe_iteration_level == self.maximum_iteration {
                            // 100 -> 0 + 1 = 1, 101 -> 100 + 1 = 101
                            *probe_iteration_level = ((*probe_iteration_level - 1) / self.data_storage_interval) * self.data_storage_interval + 1;
                        }
                    };
                });

            // we have now iterated all the values, we need to update those which skipped too quickly
            self.min_valid_iteration = *valid_iterations.iter().min_by_key(|&value| {
                if *value > 0 {
                    *value
                } else {
                    usize::MAX
                }
            }).unwrap();

            // this would indicate that no more of the probes are bad
            if self.min_valid_iteration != next_probe_check_value || next_probe_check_value == 1 {
                // println!("{:?}, {}, {}, {}", valid_iterations, self.min_valid_iteration, current_probe_check_value, next_probe_check_value);
                break;
            } else {
                current_probe_check_value = next_probe_check_value;

                let test_val = max(
                    ((self.min_valid_iteration as f32 * self.valid_iteration_probe_multiplier) as usize / self.data_storage_interval) * self.data_storage_interval, 
                    self.data_storage_interval);
    
                next_probe_check_value = if current_probe_check_value > test_val {
                    current_probe_check_value - test_val
                } else {
                    1
                };
                // println!("{:?}, {}, {}, {}", valid_iterations, self.min_valid_iteration, current_probe_check_value, next_probe_check_value);
            }
        }
    }

    pub fn check_approximation(&mut self, 
        delta_top_left_mantissa: ComplexFixed<f64>, 
        delta_top_left_exponent: i32, 
        cos_rotate: f64, 
        sin_rotate: f64, 
        delta_pixel: f64,
        image_width: usize,
        image_height: usize,
        center_reference: &Reference,
        series_validation_counter: &Arc<AtomicUsize>) {

        if !self.enabled {
            self.min_valid_iteration = 1;
            self.max_valid_iteration = 1;
            
            series_validation_counter.store(2, Ordering::SeqCst);
            return;
        };

        series_validation_counter.store(0, Ordering::SeqCst);
        self.calculate_probes(image_width, image_height, cos_rotate, sin_rotate, delta_top_left_mantissa, delta_top_left_exponent, delta_pixel);

        let mut valid_iterations = vec![0; self.probe_sampling * self.probe_sampling];

        let test_val = max(
            ((self.min_valid_iteration as f32 * self.valid_iteration_probe_multiplier) as usize / self.data_storage_interval) * self.data_storage_interval, 
            (1000 / self.data_storage_interval) * self.data_storage_interval);

        let first_check_probes = vec![0, self.probe_sampling - 1, self.probe_sampling * (self.probe_sampling - 1), self.probe_sampling * self.probe_sampling - 1];

        // The check value here is much larger - we want to make sure that our assumptions are correct
        self.iterate_probes(center_reference, &mut valid_iterations, Some(&first_check_probes), test_val, 10 * test_val);

        // self.valid_iterations = valid_iterations.clone();
        // self.print_probe_iteration_buffer();

        series_validation_counter.fetch_add(1, Ordering::Relaxed);

        self.min_valid_iteration = self.data_storage_interval * ((self.min_valid_iteration - 1) / self.data_storage_interval) + 1;

        if first_check_probes.len() != valid_iterations.len() {
            let test_val = max(
                ((self.min_valid_iteration as f32 * self.valid_iteration_probe_multiplier) as usize / self.data_storage_interval) * self.data_storage_interval, 
                (1000 / self.data_storage_interval) * self.data_storage_interval);
    
            let next_check_probes = (0..(self.probe_sampling * self.probe_sampling)).flat_map(|index| {
                if !first_check_probes.contains(&index) {
                    vec![index]
                } else {
                    Vec::new()
                }
            }).collect::<Vec<usize>>();
    
            // Check value here is much smaller, because we have checked that the corner probes work well we assume that the
            // rest of the skip will be relatively constant
            self.iterate_probes(center_reference, &mut valid_iterations, Some(&next_check_probes), test_val, test_val);
        }

        self.valid_iterations = valid_iterations;
        self.interpolate_probes();

        self.max_valid_iteration = if self.tiled {
            *self.valid_interpolation.iter().max().unwrap()
        } else {
            self.min_valid_iteration
        };
        
        // self.print_probe_iteration_buffer();
        // self.print_probe_interpolation_buffer();

        series_validation_counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn print_probe_iteration_buffer(&self) {
        println!("\nProbe iteration buffer:");
        let temp_size = self.probe_sampling;
        for i in 0..temp_size {
            let test = &self.valid_iterations[(i * temp_size)..((i + 1) * temp_size)];
            print!("[");

            for element in test {
                print!("{:>8},", element);
            }

            print!("\x08]\n");
        }
    }

    pub fn print_probe_interpolation_buffer(&self) {
        println!("\nProbe interpolation buffer:");
        let temp_size = self.probe_sampling - 1;
        for i in 0..temp_size {
            let test = &self.valid_interpolation[(i * temp_size)..((i + 1) * temp_size)];
            print!("[");

            for element in test {
                print!("{:>8},", element);
            }

            print!("\x08]\n");
        }
    }

    pub fn interpolate_probes(&mut self) {
        for j in 0..(self.probe_sampling - 1) {
            for i in 0..(self.probe_sampling - 1) {
                // this is the index into the main array
                let index = j * self.probe_sampling + i;

                let min_interpolation = *[self.valid_iterations[index], 
                    self.valid_iterations[index + 1], 
                    self.valid_iterations[index + self.probe_sampling], 
                    self.valid_iterations[index + self.probe_sampling + 1]].iter().min().unwrap();

                self.valid_interpolation.push(min_interpolation);
            }
        }
    }

    pub fn add_probe(&mut self, delta_probe: ComplexExtended) {
        // here we will need to check to make sure we are still at the first iteration, or use perturbation to go forward
        self.probe_start.push(delta_probe);

        let mut current_value = delta_probe;

        let mut delta_n = Vec::with_capacity(self.order + 1);
        let mut delta_derivative_n = Vec::with_capacity(self.order + 1);

        // The first element will be 1, in order for the derivative to be calculated
        delta_n.push(current_value);
        delta_derivative_n.push(ComplexExtended::new2(1.0, 0.0, 0));

        for i in 1..=self.order {
            delta_derivative_n.push(current_value * (i + 1) as f64);
            current_value *= delta_probe;
            delta_n.push(current_value);
        }

        self.approximation_probes.push(delta_n);
        self.approximation_probes_derivative.push(delta_derivative_n);
    }

    pub fn evaluate(&self, point_delta: ComplexExtended, iteration: usize) -> ComplexExtended {
        if iteration == 1 {
            return point_delta;
        }

        // 101 -> 100 / 100 = 1, 1 -> 0 / 100 = 0, 201 -> 200 / 100 = 2
        let new_coefficients = &self.coefficients[(iteration - 1) / self.data_storage_interval];

        // Horner's rule
        let mut approximation = new_coefficients[self.order];

        for coefficient in new_coefficients[1..self.order].iter().rev() {
            approximation *= point_delta;
            approximation += *coefficient;
        }

        approximation *= point_delta;
        approximation.reduce();
        approximation
    }

    pub fn evaluate_derivative(&self, point_delta: ComplexExtended, iteration: usize) -> ComplexExtended {
        if iteration == 1 {
            return ComplexExtended::new2(1.0, 0.0, 0);
        }

        // 101 -> 100 / 100 = 1, 1 -> 0 / 100 = 0, 201 -> 200 / 100 = 2
        let new_coefficients = &self.coefficients[(iteration - 1) / self.data_storage_interval];

        // Horner's rule
        let mut approximation = new_coefficients[self.order];
        approximation *= self.order as f64;

        for k in (1..self.order).rev() {
            approximation *= point_delta;
            approximation += new_coefficients[k] * k as f64;
        }

        approximation.reduce();
        approximation
    }
}
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::BufWriter;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() <= 3 {
        return;
    }

    let pixels_per_chunk = 16;

    let final_color_type = 3;

    let source_dimensions = 416;

    let mut source_chunk_dimensions = source_dimensions / pixels_per_chunk;
    if source_dimensions % pixels_per_chunk != 0 {
        source_chunk_dimensions += 1;
    }

    let target_file = File::open(&args[1]).expect("Failed to open target file");
    let target_decoder = png::Decoder::new(target_file);
    let (target_hdr, target_reader) = target_decoder.read_info().unwrap();
    let mut original_width = target_hdr.width;
    if original_width % final_color_type != 0 {
        original_width += final_color_type;
    }

    let (mut target_color_buffer, target_width, target_height) = pixelate(
        final_color_type,
        target_hdr,
        target_reader,
        pixels_per_chunk,
    );

    println!("Finished phase 1");

    let source_color_buffers = analyze_source(
        &args[3],
        final_color_type,
        pixels_per_chunk,
        source_chunk_dimensions,
    );

    println!("Finished phase 2");

    let mut source_colors = HashMap::new();

    for source_color_buffer in source_color_buffers {
        let mut current_interval = 0;
        let mut average_color = vec![0; final_color_type as usize];
        for &source_color in &source_color_buffer {
            average_color[(current_interval % final_color_type) as usize] += source_color as u32;
            current_interval += 1;
        }

        for color in &mut average_color {
            *color /= current_interval / final_color_type;
        }
        source_colors.insert(average_color, source_color_buffer);
    }

    println!("Finished phase 3");

    for interval in 0..(target_color_buffer.len() / final_color_type as usize) {
        let mut temp_target_color = vec![0; final_color_type as usize];
        let mut last_distance = 1000.0;
        for average_color in source_colors.clone() {
            let mut distance = 0.0;
            for value in 0..final_color_type {
                distance += (target_color_buffer
                    [((interval * final_color_type as usize) + value as usize) as usize]
                    as f64
                    - average_color.0[value as usize] as f64)
                    * (target_color_buffer
                        [((interval * final_color_type as usize) + value as usize) as usize]
                        as f64
                        - average_color.0[value as usize] as f64);
            }
            if distance.sqrt() < last_distance {
                last_distance = distance.sqrt();
                temp_target_color = average_color.0;
            }
        }
        for temp_interval in 0..final_color_type {
            target_color_buffer[(interval * final_color_type as usize) + temp_interval as usize] =
                (temp_target_color[temp_interval as usize]) as u8;
        }
    }

    println!("Finished phase 4");

    let mut final_buffer = vec![
        0;
        target_color_buffer.len()
            * (source_chunk_dimensions * source_chunk_dimensions) as usize
    ];

    for chunk_interval in 0..((target_color_buffer.len() * source_chunk_dimensions as usize)
        / final_color_type as usize)
    {
        let final_chunk_location = chunk_interval % (original_width / pixels_per_chunk) as usize
            + (chunk_interval
                / (original_width * source_chunk_dimensions / pixels_per_chunk) as usize
                * (original_width / pixels_per_chunk) as usize);
        let scanline_number = chunk_interval / (original_width / pixels_per_chunk) as usize
            % (source_chunk_dimensions) as usize
            * (source_chunk_dimensions) as usize;

        let mut color_key = Vec::new();

        for interval in 0..final_color_type as usize {
            color_key.push(
                target_color_buffer[final_chunk_location * final_color_type as usize + interval]
                    as u32,
            );
        }

        for pixel_interval in 0..(source_chunk_dimensions) as usize {
            for byte_interval in 0..final_color_type as usize {
                final_buffer[chunk_interval
                    * source_chunk_dimensions as usize
                    * final_color_type as usize
                    + pixel_interval * final_color_type as usize
                    + byte_interval] = source_colors[&color_key][pixel_interval
                    * final_color_type as usize
                    + byte_interval
                    + scanline_number * final_color_type as usize];
            }
        }
    }

    println!("Finished phase 5");

    let raw_file = File::create(&args[2]).expect("Failed to create raw file");
    let file_writer = &mut BufWriter::new(raw_file);
    let mut encoder = png::Encoder::new(
        file_writer,
        target_width * source_chunk_dimensions,
        target_height * source_chunk_dimensions,
    );
    encoder.set_color(png::ColorType::RGB);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();

    writer.write_image_data(&final_buffer).unwrap();
}

struct RGBA {
    r: u32,
    g: u32,
    b: u32,
    a: u32,
}

fn pixelate(
    final_bytes_per_pixel: u32,
    hdr: png::OutputInfo,
    mut reader: png::Reader<File>,
    pixels_per_chunk: u32,
) -> (Vec<u8>, u32, u32) {
    let bytes_per_pixel = match hdr.color_type {
        png::ColorType::RGB => 3,
        png::ColorType::RGBA => 4,
        _ => panic!("Unrecognized PNG byte format"),
    };
    let mut buffer = vec![0; (hdr.width * hdr.height * bytes_per_pixel) as usize];
    reader
        .next_frame(&mut buffer)
        .expect("Failed to decode frame");
    let mut chunks_in_width = hdr.width / pixels_per_chunk;
    if hdr.width % pixels_per_chunk != 0 {
        chunks_in_width += 1;
    }
    let mut chunks_in_height = hdr.height / pixels_per_chunk;
    if hdr.height % pixels_per_chunk != 0 {
        chunks_in_height += 1;
    }

    let mut temp_color_buffer = Vec::new();
    for _ in 0..(chunks_in_width * chunks_in_height) {
        temp_color_buffer.push(RGBA {
            r: 0,
            g: 0,
            b: 0,
            a: 0,
        });
    }

    let mut current_interval = 0;
    for current_byte in buffer {
        let buffer_index = current_interval / bytes_per_pixel / pixels_per_chunk % chunks_in_width
            + (current_interval / hdr.width / bytes_per_pixel / pixels_per_chunk) * chunks_in_width;
        match current_interval % bytes_per_pixel {
            0 => temp_color_buffer[buffer_index as usize].r += current_byte as u32,
            1 => temp_color_buffer[buffer_index as usize].g += current_byte as u32,
            2 => temp_color_buffer[buffer_index as usize].b += current_byte as u32,
            3 => temp_color_buffer[buffer_index as usize].a += current_byte as u32,
            _ => (),
        }
        current_interval += 1;
    }

    let mut color_buffer: Vec<u8> =
        vec![0; (chunks_in_height * chunks_in_width * final_bytes_per_pixel) as usize];
    current_interval = 0;
    for byte in &mut temp_color_buffer {
        for rgba_interval in 0..final_bytes_per_pixel {
            color_buffer[(current_interval + rgba_interval) as usize] = match rgba_interval {
                0 => (byte.r / (pixels_per_chunk * pixels_per_chunk)) as u8,
                1 => (byte.g / (pixels_per_chunk * pixels_per_chunk)) as u8,
                2 => (byte.b / (pixels_per_chunk * pixels_per_chunk)) as u8,
                3 => (byte.a / (pixels_per_chunk * pixels_per_chunk)) as u8,
                _ => 0,
            };
        }
        current_interval += final_bytes_per_pixel;
    }

    (color_buffer, chunks_in_width, chunks_in_height)
}

fn analyze_source(
    dir_path: &str,
    final_color_type: u32,
    pixels_per_chunk: u32,
    source_chunk_dimensions: u32,
) -> Vec<Vec<u8>> {
    let mut source_color_buffers = Vec::new();

    let source_dir = fs::read_dir(dir_path).unwrap();

    for source_result in source_dir {
        let (source_color_buffer, source_chunks_width, source_chunks_height) =
            if let Ok(source_path) = source_result {
                let source_file =
                    File::open(source_path.path()).expect("Failed to open source file");
                let source_decoder = png::Decoder::new(source_file);
                let (source_hdr, source_reader) =
                    if let Ok(source_decoded) = source_decoder.read_info() {
                        source_decoded
                    } else {
                        continue;
                    };

                pixelate(
                    final_color_type,
                    source_hdr,
                    source_reader,
                    pixels_per_chunk,
                )
            } else {
                continue;
            };
        if source_chunks_width == source_chunks_height
            && source_chunks_width == source_chunk_dimensions
        {
            source_color_buffers.push(source_color_buffer);
        }
    }

    source_color_buffers
}

#[cfg(test)]
mod tests {
    //use super::*;

    #[test]
    fn test_pixelate() {}
}

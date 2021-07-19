use png::OutputInfo;
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::BufWriter;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() <= 3 {
        return;
    }

    let mut target_pixels_per_chunk = 4;
    let mut source_pixels_per_chunk = 52;
    let source_dimensions = 416;

    let final_color_type = 3;

    source_pixels_per_chunk = pixels_per_chunk_gcf(
        source_pixels_per_chunk,
        source_dimensions,
        source_dimensions,
    );

    let source_chunk_dimensions = source_dimensions / source_pixels_per_chunk;

    let target_file = File::open(&args[1]).expect("Failed to open target file");
    let target_decoder = png::Decoder::new(target_file);
    let (target_hdr, target_reader) = target_decoder.read_info().unwrap();

    target_pixels_per_chunk =
        pixels_per_chunk_gcf(target_pixels_per_chunk, target_hdr.width, target_hdr.height);

    let (target_color_buffer, target_width, target_height) = pixelate(
        final_color_type,
        target_hdr,
        target_reader,
        target_pixels_per_chunk,
    );

    println!("Finished phase 1 out of 5");

    let source_color_buffers = analyze_source(
        &args[3],
        final_color_type,
        source_pixels_per_chunk,
        source_chunk_dimensions,
    );

    println!("Finished phase 2 out of 5");

    let source_colors = generate_source_map(source_color_buffers, final_color_type);

    println!("Finished phase 3 out of 5");

    let target_color_buffer =
        apply_source_palette(target_color_buffer, final_color_type, source_colors.clone());

    println!("Finished phase 4 out of 5");

    let final_buffer = construct_mosaic(
        target_color_buffer,
        source_chunk_dimensions,
        target_width,
        target_height,
        final_color_type,
        source_colors,
    );

    println!("Finished phase 5 out of 5");

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

fn pixels_per_chunk_gcf(mut pixels_per_chunk: u32, width: u32, height: u32) -> u32 {
    loop {
        if width % pixels_per_chunk == 0 && height % pixels_per_chunk == 0 {
            break pixels_per_chunk;
        }
        if pixels_per_chunk > width || pixels_per_chunk > height || pixels_per_chunk == 0 {
            panic!("Pixels per chunk cannot be larger than image width/length");
        }
        pixels_per_chunk += 1;
    }
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
    let chunks_in_width = hdr.width / pixels_per_chunk;
    let chunks_in_height = hdr.height / pixels_per_chunk;

    let temp_color_buffer = accumulate_pixels(
        chunks_in_width,
        chunks_in_height,
        final_bytes_per_pixel,
        bytes_per_pixel,
        buffer,
        pixels_per_chunk,
        hdr,
    );

    let color_buffer = reduce_pixels(
        chunks_in_width,
        chunks_in_height,
        final_bytes_per_pixel,
        temp_color_buffer,
        pixels_per_chunk,
    );

    (color_buffer, chunks_in_width, chunks_in_height)
}

fn accumulate_pixels(
    chunks_in_width: u32,
    chunks_in_height: u32,
    final_bytes_per_pixel: u32,
    bytes_per_pixel: u32,
    buffer: Vec<u8>,
    pixels_per_chunk: u32,
    hdr: OutputInfo,
) -> Vec<u32> {
    let mut temp_color_buffer =
        vec![0; (chunks_in_height * chunks_in_width * final_bytes_per_pixel) as usize];

    for (current_interval, current_byte) in buffer.into_iter().enumerate() {
        let buffer_index = current_interval as u32 / bytes_per_pixel / pixels_per_chunk
            % chunks_in_width
            + (current_interval as u32 / hdr.width / bytes_per_pixel / pixels_per_chunk)
                * chunks_in_width;
        let pixel_interval = match current_interval as u32 % bytes_per_pixel {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => {
                continue;
            }
            _ => panic!("Incompatible pixel size"),
        };
        temp_color_buffer[(buffer_index * final_bytes_per_pixel + pixel_interval) as usize] +=
            current_byte as u32;
    }
    temp_color_buffer
}

fn reduce_pixels(
    chunks_in_width: u32,
    chunks_in_height: u32,
    final_bytes_per_pixel: u32,
    temp_color_buffer: Vec<u32>,
    pixels_per_chunk: u32,
) -> Vec<u8> {
    let mut color_buffer: Vec<u8> =
        vec![0; (chunks_in_height * chunks_in_width * final_bytes_per_pixel) as usize];
    for (current_interval, byte) in temp_color_buffer.into_iter().enumerate() {
        color_buffer[current_interval] = (byte / pixels_per_chunk / pixels_per_chunk) as u8;
    }

    color_buffer
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
        let (source_color_buffer, _, _) = if let Ok(source_path) = source_result {
            let source_file = File::open(source_path.path()).expect("Failed to open source file");
            let source_decoder = png::Decoder::new(source_file);
            let (source_hdr, source_reader) = if let Ok(source_decoded) = source_decoder.read_info()
            {
                source_decoded
            } else {
                continue;
            };
            if source_hdr.width != source_hdr.height
                || source_hdr.width != source_chunk_dimensions * pixels_per_chunk
            {
                continue;
            }
            pixelate(
                final_color_type,
                source_hdr,
                source_reader,
                pixels_per_chunk,
            )
        } else {
            continue;
        };
        source_color_buffers.push(source_color_buffer);
    }

    source_color_buffers
}

fn generate_source_map(
    source_color_buffers: Vec<Vec<u8>>,
    final_color_type: u32,
) -> HashMap<Vec<u32>, Vec<u8>> {
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
    source_colors
}

fn apply_source_palette(
    mut target_color_buffer: Vec<u8>,
    final_color_type: u32,
    source_colors: HashMap<Vec<u32>, Vec<u8>>,
) -> Vec<u8> {
    for interval in 0..(target_color_buffer.len() / final_color_type as usize) {
        let mut temp_target_color = vec![0; final_color_type as usize];
        let mut last_distance = 1000.0;
        for (average_color, _) in source_colors.clone() {
            let mut distance = 0.0;
            for value in 0..final_color_type {
                distance += (target_color_buffer
                    [((interval * final_color_type as usize) + value as usize) as usize]
                    as f64
                    - average_color[value as usize] as f64)
                    * (target_color_buffer
                        [((interval * final_color_type as usize) + value as usize) as usize]
                        as f64
                        - average_color[value as usize] as f64);
            }
            if distance.sqrt() < last_distance {
                last_distance = distance.sqrt();
                temp_target_color = average_color;
            }
        }
        for temp_interval in 0..final_color_type {
            target_color_buffer[(interval * final_color_type as usize) + temp_interval as usize] =
                (temp_target_color[temp_interval as usize]) as u8;
        }
    }
    target_color_buffer
}

fn construct_mosaic(
    target_color_buffer: Vec<u8>,
    source_chunk_dimensions: u32,
    target_width: u32,
    target_height: u32,
    final_color_type: u32,
    source_colors: HashMap<Vec<u32>, Vec<u8>>,
) -> Vec<u8> {
    let mut final_buffer = vec![
        0;
        target_color_buffer.len()
            * (source_chunk_dimensions * source_chunk_dimensions) as usize
    ];

    for chunk_interval in 0..((target_height * target_width * source_chunk_dimensions) as usize) {
        let final_chunk_location = chunk_interval % target_width as usize
            + (chunk_interval / (target_width * source_chunk_dimensions) as usize
                * target_width as usize);
        let scanline_number = chunk_interval / target_width as usize
            % source_chunk_dimensions as usize
            * source_chunk_dimensions as usize;

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
    final_buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pixels_per_chunk_gcf() {
        assert_eq!(pixels_per_chunk_gcf(5, 101, 101), 101);
        assert_eq!(pixels_per_chunk_gcf(4, 2048, 1024), 4);
        assert_eq!(pixels_per_chunk_gcf(7, 1000, 2000), 8);
    }

    #[test]
    #[should_panic]
    fn panic_pixels_per_chunk_gcf() {
        pixels_per_chunk_gcf(417, 2000, 17);
        pixels_per_chunk_gcf(0, 200, 400);
    }

    #[test]
    fn test_accumulate_pixels() {}
}

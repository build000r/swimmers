const HSL_RGB_SECTORS: [[usize; 3]; 6] = [
    [0, 1, 2],
    [1, 0, 2],
    [2, 0, 1],
    [2, 1, 0],
    [1, 2, 0],
    [0, 2, 1],
];

pub fn hsl_to_rgb(hue: f64, saturation: f64, lightness: f64) -> (u8, u8, u8) {
    let chroma = hsl_chroma(saturation, lightness);
    let h_prime = wrap_hue(hue) / 60.0;
    let secondary = hsl_secondary_component(chroma, h_prime);
    let components = [chroma, secondary, 0.0];
    let [red, green, blue] = HSL_RGB_SECTORS[h_prime.floor() as usize % HSL_RGB_SECTORS.len()];
    let match_value = lightness - chroma / 2.0;

    (
        hsl_component_to_byte(components[red], match_value),
        hsl_component_to_byte(components[green], match_value),
        hsl_component_to_byte(components[blue], match_value),
    )
}

fn wrap_hue(hue: f64) -> f64 {
    let wrapped = hue % 360.0;
    if wrapped < 0.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

fn hsl_chroma(saturation: f64, lightness: f64) -> f64 {
    (1.0 - (2.0 * lightness - 1.0).abs()) * saturation
}

fn hsl_secondary_component(chroma: f64, h_prime: f64) -> f64 {
    chroma * (1.0 - ((h_prime % 2.0) - 1.0).abs())
}

fn hsl_component_to_byte(component: f64, match_value: f64) -> u8 {
    ((component + match_value).clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_to_rgb_primary_hues() {
        assert_eq!(hsl_to_rgb(0.0, 1.0, 0.5), (255, 0, 0));
        assert_eq!(hsl_to_rgb(60.0, 1.0, 0.5), (255, 255, 0));
        assert_eq!(hsl_to_rgb(120.0, 1.0, 0.5), (0, 255, 0));
        assert_eq!(hsl_to_rgb(180.0, 1.0, 0.5), (0, 255, 255));
        assert_eq!(hsl_to_rgb(240.0, 1.0, 0.5), (0, 0, 255));
        assert_eq!(hsl_to_rgb(300.0, 1.0, 0.5), (255, 0, 255));
    }

    #[test]
    fn hsl_to_rgb_zero_saturation_is_gray_with_byte_rounding() {
        assert_eq!(hsl_to_rgb(37.0, 0.0, 0.5), (128, 128, 128));
        assert_eq!(hsl_to_rgb(222.0, 0.0, 0.25), (64, 64, 64));
    }

    #[test]
    fn hsl_to_rgb_wraps_hue() {
        assert_eq!(hsl_to_rgb(420.0, 1.0, 0.5), hsl_to_rgb(60.0, 1.0, 0.5));
        assert_eq!(hsl_to_rgb(-120.0, 1.0, 0.5), hsl_to_rgb(240.0, 1.0, 0.5));
    }
}

//! Great-circle distance, shared by the airport circles.

pub const EARTH_RADIUS_NM: f64 = 3440.065;

/// Great-circle distance in nautical miles (haversine).
pub fn distance_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_NM * a.sqrt().asin()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_symmetric() {
        let a = distance_nm(41.2753, 28.7519, 40.8986, 29.3092);
        let b = distance_nm(40.8986, 29.3092, 41.2753, 28.7519);
        assert!((a - b).abs() < 1e-9);
    }

    #[test]
    fn one_degree_of_latitude_is_about_60_nm() {
        assert!((distance_nm(41.0, 28.0, 42.0, 28.0) - 60.04).abs() < 0.01);
    }
}

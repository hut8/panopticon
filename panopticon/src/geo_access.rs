use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{info, warn};

/// Cached GPS position (latitude, longitude).
type GpsPosition = Arc<RwLock<Option<(f64, f64)>>>;

/// Geo-based access control: allows IPs that geolocate within a configurable
/// radius of the device's GPS position.
#[derive(Clone)]
pub struct GeoAccess {
    reader: Option<Arc<maxminddb::Reader<Vec<u8>>>>,
    gps_position: GpsPosition,
    radius_miles: f64,
    gpsd_host: String,
    gpsd_port: u16,
}

impl GeoAccess {
    /// Initialize the GeoAccess subsystem. Downloads the GeoIP database if it
    /// doesn't exist on disk. Falls back to whitelist-only if the DB can't be
    /// loaded or downloaded.
    pub async fn init() -> Self {
        let db_path = std::env::var("GEOIP_DB_PATH")
            .unwrap_or_else(|_| "/var/lib/panopticon/GeoLite2-City.mmdb".to_string());
        let radius_miles: f64 = std::env::var("GEO_RADIUS_MILES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100.0);
        let gpsd_host = std::env::var("GPSD_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let gpsd_port: u16 = std::env::var("GPSD_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2947);

        // Download the DB if it doesn't exist on disk.
        if !Path::new(&db_path).exists() {
            info!(path = %db_path, "GeoIP database not found, downloading");
            if let Err(e) = download_geoip_db(&db_path).await {
                warn!(error = %e, "Failed to download GeoIP database — geo access disabled, whitelist-only");
            }
        }

        let reader = match maxminddb::Reader::open_readfile(&db_path) {
            Ok(r) => {
                info!(path = %db_path, "Loaded GeoIP database");
                Some(Arc::new(r))
            }
            Err(e) => {
                warn!(path = %db_path, error = %e, "GeoIP database not available — geo access disabled, whitelist-only");
                None
            }
        };

        GeoAccess {
            reader,
            gps_position: Arc::new(RwLock::new(None)),
            radius_miles,
            gpsd_host,
            gpsd_port,
        }
    }

    /// Spawn a background task that connects to gpsd and maintains the cached
    /// GPS position. Reconnects with 10s backoff on failure.
    pub fn spawn_gpsd_task(&self) {
        let position = self.gps_position.clone();
        let host = self.gpsd_host.clone();
        let port = self.gpsd_port;

        tokio::spawn(async move {
            loop {
                if let Err(e) = gpsd_session(&host, port, &position).await {
                    warn!(error = %e, "gpsd connection lost");
                }
                // Clear position on disconnect — no stale data.
                *position.write().await = None;
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        });
    }

    /// Check whether the given IP geolocates within the configured radius of
    /// the device's current GPS position. Returns `false` if any data is
    /// unavailable (no DB, no fix, IP not found).
    pub async fn is_within_radius(&self, ip: IpAddr) -> bool {
        let reader = match &self.reader {
            Some(r) => r,
            None => return false,
        };

        let gps = match *self.gps_position.read().await {
            Some(pos) => pos,
            None => return false,
        };

        // Look up the IP in the GeoIP database.
        let city: maxminddb::geoip2::City = match reader.lookup(ip) {
            Ok(c) => c,
            Err(_) => return false,
        };

        let location = match city.location {
            Some(ref loc) => loc,
            None => return false,
        };

        let (ip_lat, ip_lon) = match (location.latitude, location.longitude) {
            (Some(lat), Some(lon)) => (lat, lon),
            _ => return false,
        };

        let distance = haversine_miles(gps.0, gps.1, ip_lat, ip_lon);
        distance <= self.radius_miles
    }
}

const GEOIP_DB_URL: &str = "https://cdn.jsdelivr.net/npm/geolite2-city/GeoLite2-City.mmdb.gz";

/// Download and decompress the GeoLite2-City database.
async fn download_geoip_db(dest: &str) -> anyhow::Result<()> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let response = reqwest::get(GEOIP_DB_URL).await?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status} from GeoIP download");
    }

    let compressed = response.bytes().await?;
    info!(
        bytes = compressed.len(),
        "Downloaded GeoIP database (compressed)"
    );

    // Decompress gzip.
    let mut decoder = GzDecoder::new(&compressed[..]);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;

    // Ensure parent directory exists.
    if let Some(parent) = Path::new(dest).parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(dest, &decompressed)?;
    info!(path = %dest, bytes = decompressed.len(), "Wrote GeoIP database");

    Ok(())
}

/// Connect to gpsd, send the WATCH command, and stream TPV reports until the
/// connection drops.
async fn gpsd_session(host: &str, port: u16, position: &GpsPosition) -> anyhow::Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpStream;

    let addr = format!("{host}:{port}");
    info!(addr = %addr, "Connecting to gpsd");
    let mut stream = TcpStream::connect(&addr).await?;

    // Enable JSON watch mode.
    stream
        .write_all(b"?WATCH={\"enable\":true,\"json\":true}\n")
        .await?;
    info!("gpsd WATCH enabled");

    let reader = BufReader::new(stream);
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        // We only care about TPV (Time-Position-Velocity) reports.
        if !line.contains("\"class\":\"TPV\"") {
            continue;
        }

        // Parse just the fields we need — avoid pulling in a full struct.
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let lat = v.get("lat").and_then(|v| v.as_f64());
        let lon = v.get("lon").and_then(|v| v.as_f64());

        if let (Some(lat), Some(lon)) = (lat, lon) {
            let mut pos = position.write().await;
            *pos = Some((lat, lon));
            info!(lat, lon, "GPS position updated");
        }
    }

    Ok(())
}

/// Haversine distance between two (lat, lon) points in miles.
fn haversine_miles(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const EARTH_RADIUS_MILES: f64 = 3958.8;

    let (lat1, lon1) = (lat1.to_radians(), lon1.to_radians());
    let (lat2, lon2) = (lat2.to_radians(), lon2.to_radians());

    let dlat = lat2 - lat1;
    let dlon = lon2 - lon1;

    let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();

    EARTH_RADIUS_MILES * c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haversine_nyc_to_la() {
        // NYC (40.7128, -74.0060) → LA (34.0522, -118.2437) ≈ 2,451 miles
        let d = haversine_miles(40.7128, -74.0060, 34.0522, -118.2437);
        assert!((d - 2451.0).abs() < 10.0, "NYC→LA was {d} miles");
    }

    #[test]
    fn haversine_london_to_paris() {
        // London (51.5074, -0.1278) → Paris (48.8566, 2.3522) ≈ 213 miles
        let d = haversine_miles(51.5074, -0.1278, 48.8566, 2.3522);
        assert!((d - 213.0).abs() < 5.0, "London→Paris was {d} miles");
    }

    #[test]
    fn haversine_same_point() {
        let d = haversine_miles(40.0, -74.0, 40.0, -74.0);
        assert!(d.abs() < 0.001, "Same point should be ~0, got {d}");
    }

    #[test]
    fn haversine_short_distance() {
        // Two points about 50 miles apart (roughly 0.72° latitude)
        let d = haversine_miles(40.0, -74.0, 40.72, -74.0);
        assert!((d - 49.7).abs() < 2.0, "Short distance was {d} miles");
    }
}

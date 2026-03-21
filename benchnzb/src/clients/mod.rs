pub mod rustnzb;
pub mod sabnzbd;

#[derive(Default)]
pub struct StageTiming {
    pub download_sec: f64,
    pub par2_sec: f64,
    pub unpack_sec: f64,
}

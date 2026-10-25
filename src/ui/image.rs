//! Local PNG/JPG theme backgrounds through WIC (decoded at most 1920 px wide,
//! like the old `DecodePixelWidth`).

use windows::Win32::Graphics::Direct2D::{ID2D1Bitmap, ID2D1RenderTarget};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::core::HSTRING;

pub fn load(target: &ID2D1RenderTarget, path: &str) -> Option<ID2D1Bitmap> {
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let decoder = factory
            .CreateDecoderFromFilename(&HSTRING::from(path), None, windows::Win32::Foundation::GENERIC_READ, WICDecodeMetadataCacheOnLoad)
            .ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h).ok()?;
        let source: IWICBitmapSource = if w > 1920 {
            let scaler = factory.CreateBitmapScaler().ok()?;
            let nh = ((h as f64) * 1920.0 / w as f64).round().max(1.0) as u32;
            scaler.Initialize(&frame, 1920, nh, WICBitmapInterpolationModeFant).ok()?;
            scaler.into()
        } else {
            frame.into()
        };
        let converter = factory.CreateFormatConverter().ok()?;
        converter
            .Initialize(&source, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
            .ok()?;
        target.CreateBitmapFromWicBitmap(&converter, None).ok()
    }
}

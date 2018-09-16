use std::ffi::OsStr;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;
use std::mem::{self, size_of};
use std::slice::from_raw_parts;
use std::io::prelude::*;

use winapi::shared::minwindef::{TRUE, FALSE, LPVOID, WORD, DWORD};
use winapi::shared::windef::{HWND, HDC, HGDIOBJ, RECT, LPRECT};
use winapi::um::winuser::{GetClientRect, GetWindowRect, GetWindowLongPtrA, GWL_STYLE, WS_POPUP, GetDC, FindWindowW, GetDesktopWindow};
use winapi::um::wingdi::*;

use failure::{Error, err_msg};

use image::{self, DynamicImage};

pub struct Screenshoter;

impl Screenshoter {
    pub fn get_window_dimensions(&self, window_title: &str) -> Result<(i32, i32), Error> {
        // Get window
        let handle = self.get_window_handle(window_title)?;

        // Get window dimensions
        unsafe {
            let mut window_size: RECT = mem::uninitialized();
            if GetClientRect(handle, &mut window_size as LPRECT) == FALSE {
                panic!("Unable to get window dimensions");
            }

            let width = window_size.right - window_size.left;
            let height = window_size.bottom - window_size.top;

            Ok((width, height))
        }
    }

    pub fn screenshot_window(&self, window_title: &str, x: i32, y: i32, width: i32, height: i32) -> Result<DynamicImage, Error> {
        // Get window handles
        let mut handle = self.get_window_handle(window_title)?;

        // Check if it's a fullscreen window
        let fullscreen = unsafe {
            // Get the dimensions of the desktop and the window
            let mut a: RECT = mem::uninitialized();
            let mut b: RECT = mem::uninitialized();
            GetClientRect(handle, &mut a as LPRECT);
            GetWindowRect(GetDesktopWindow(), &mut b as LPRECT);

            // Get the window styles
            let window_style = GetWindowLongPtrA(handle, GWL_STYLE);

            // If the resolutions are the exact same than it might be a fullscreen window
            let same_res = a.left == b.left && a.top == b.top && a.right == b.right && a.bottom == b.bottom;

            // If the window is not a popup window and has the same resolution, it is fullscreened
            same_res && (window_style as usize & WS_POPUP as usize) == 0
        };

        // If it's fullscreen, than we need to capture the desktop buffer since GDI can't access
        // application buffers for exclusive fullscreen windows. 
        if fullscreen {
            handle = null_mut();
        }

        // Get the DC for whichever window we are targeting
        let window_dc = unsafe {
            GetDC(handle)
                .as_mut()
                .map(|p| p as HDC)
                .ok_or_else(|| err_msg("Couldn't get display context"))?
        };

        // Create an in memory bmp by copying a window
        let bmp = unsafe {
            // Create temporary destination targets
            let h_dc_mem = CreateCompatibleDC(window_dc)
                .as_mut()
                .ok_or_else(|| err_msg("Couldn't create a compatible DC"))?;
            let h_bitmap = CreateCompatibleBitmap(window_dc, width, height)
                .as_mut()
                .ok_or_else(|| err_msg("Couldn't create a compatible destination bitmap"))?;
            let old_object = SelectObject(h_dc_mem, h_bitmap as *mut _ as HGDIOBJ);

            // Copy the contents of the window into our temporary destinations
            debug_assert_eq!(BitBlt(h_dc_mem, 0, 0, width, height, window_dc, x, y, SRCCOPY), TRUE);

            // Setup bitmap data headers before we copy it out
            let mut bitmap_info: BITMAPINFO = mem::uninitialized();
            bitmap_info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            debug_assert_ne!(GetDIBits(h_dc_mem, h_bitmap, 0, 0, null_mut(), &mut bitmap_info, DIB_RGB_COLORS), 0);
            bitmap_info.bmiHeader.biBitCount = 32;                                  // 32 bit depth
            bitmap_info.bmiHeader.biCompression = BI_RGB;                           // Uncompressed RGB
            bitmap_info.bmiHeader.biHeight = bitmap_info.bmiHeader.biHeight.abs();  // English order

            // Download the bitmap data from our temporary destinations into a byte buffer
            let mut pixel_data: Vec<u8> = vec![0; bitmap_info.bmiHeader.biSizeImage as usize];
            let ret = GetDIBits(h_dc_mem, h_bitmap, 0, bitmap_info.bmiHeader.biHeight as u32, pixel_data.as_mut_ptr() as LPVOID, &mut bitmap_info, DIB_RGB_COLORS);
            debug_assert_ne!(ret, 0);

            // Cleanup the temporary GDI objects
            debug_assert!(SelectObject(h_dc_mem, old_object) != null_mut());
            debug_assert!(DeleteObject(h_bitmap as *mut _ as HGDIOBJ) == TRUE);
            debug_assert!(DeleteDC(h_dc_mem) == TRUE);

            // Write the bitmap data as a in memory bmp
            self.output_bitmap(&bitmap_info, &pixel_data)
        };

        // Load the image into piston's DynamicImage format for easier processing and return it
        let image = image::load_from_memory(&bmp)?;
        Ok(image)
    }

    /// Get a window handle given a window title
    fn get_window_handle(&self, title: &str) -> Result<HWND, Error> {
        // Convert the rust utf-8 string into a utf-16 windows string
        let wide_str: Vec<u16> = OsStr::new(title).encode_wide().chain(once(0)).collect();

        // Call FindWindowW, passing it the wide string to get the matching window handle
        unsafe {
            FindWindowW(null_mut(), wide_str.as_ptr())
                .as_mut()
                .map(|p| p as HWND)
                .ok_or_else(|| err_msg("Couldn't find window"))
        }
    }

    /// Output a bitmap to a buffer
    fn output_bitmap(&self, bitmap_info: &BITMAPINFO, pixel_data: &[u8]) -> Vec<u8> {
        // Make a bitmap file header
        let mut file_header: BITMAPFILEHEADER = unsafe { mem::uninitialized() };
        file_header.bfType = (('M' as u32) << 8 | 'B' as u32) as WORD;
        file_header.bfSize = (size_of::<BITMAPFILEHEADER>() + size_of::<BITMAPINFOHEADER>() + pixel_data.len()) as DWORD;
        file_header.bfReserved1 = 0;
        file_header.bfReserved2 = 0;
        file_header.bfOffBits = (size_of::<BITMAPFILEHEADER>() + size_of::<BITMAPINFOHEADER>()) as DWORD;

        // Serialize the struct as a slice of bytes
        let p: *const BITMAPFILEHEADER = &file_header;
        let file_header_bytes: &[u8] = unsafe { from_raw_parts(p as *const u8, size_of::<BITMAPFILEHEADER>()) };

        // Serialize the struct as a slice of bytes
        let p: *const BITMAPINFOHEADER = &bitmap_info.bmiHeader;
        let bitmap_info_header_bytes: &[u8] = unsafe { from_raw_parts(p as *const u8, size_of::<BITMAPINFOHEADER>()) };

        // Write the bitmap data to an output buffer, ready to be written directly to disk
        let mut output = Vec::with_capacity(file_header.bfSize as usize);
        output.write_all(file_header_bytes).unwrap();
        output.write_all(bitmap_info_header_bytes).unwrap();
        output.write_all(&pixel_data).unwrap();

        output
    }
}
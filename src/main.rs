extern crate failure;
extern crate image;
extern crate imageproc;
#[cfg(windows)] extern crate winapi;

use image::FilterType;
use failure::Error;

mod screenshot;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CurrentTeam {
    CT,
    T,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct GameState {
    pub current_team: Option<CurrentTeam>,
    pub num_ct: u8,
    pub num_t: u8,
}

impl Default for GameState {
    fn default() -> Self {
        GameState {
            current_team: None,
            num_ct: 0,
            num_t: 0,
        }
    }
}

fn process_game_info() -> Result<GameState, Error> {
    // We are targeting about one check a second
    // Screenshots take about 10% of the time budget, clocking in at around 97 ms, with most below that
    let recorder = screenshot::Screenshoter;
    let (width, height) = recorder.get_window_dimensions("Counter-Strike: Global Offensive")?;

    // Only capture the mini scoreboard at the top
    let alive_height = height / 20;
    let alive_width = (alive_height as f32 * 11.48) as i32;
    let x = (width / 2) - alive_width / 2;
    let screenshot = recorder.screenshot_window("Counter-Strike: Global Offensive", x, 0, alive_width, alive_height)?;

    // Do some basic pre-processing
    let half_width = alive_width as u32 / 2;
    let offset = alive_height as u32 / 2;
    let ct_side = screenshot.clone().crop(0, 0, half_width - offset, alive_height as u32).to_luma();
    let t_side  = screenshot.clone().crop(half_width + offset, 0, half_width, alive_height as u32).to_luma();

    // Edge detection
    let edges_ct = imageproc::edges::canny(&ct_side, 40.0, 100.0);
    let edges_t =  imageproc::edges::canny(&t_side, 40.0, 100.0);

    // Save
    edges_ct.save("edges_ct.bmp")?;
    edges_t.save("edges_t.bmp")?;

    Ok(GameState {
        current_team: None,
        num_ct: 0,
        num_t: 0,
    })
}

fn check_process_exists(executable: &str) -> bool {
    use std::mem::size_of;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;
    use winapi::um::psapi::{EnumProcesses, GetProcessImageFileNameW};
    use winapi::um::processthreadsapi::OpenProcess;
    use winapi::um::winnt::PROCESS_QUERY_LIMITED_INFORMATION;
    use winapi::shared::minwindef::{FALSE, DWORD};

    let executable_os = OsString::from(&executable);

    // Copy current process id's into an array
    let mut process_ids: Vec<DWORD> = vec![0; 1024];
    let mut required_size_bytes: DWORD = 0;
    unsafe {
        debug_assert_ne!(EnumProcesses(process_ids.as_mut_ptr(), 1024, &mut required_size_bytes), 0);
    }

    // Calculate process names
    let num_processes_found = required_size_bytes as usize / size_of::<DWORD>();

    // Allocate a buffer to hold the file name
    let mut image_file_name = vec![0u16; 1024];

    // Enumerate through the found processes
    for i in 0..num_processes_found {
        if process_ids[i] != 0 {
            let process_name = unsafe {
                let h_process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, process_ids[i]);
                let name_length = GetProcessImageFileNameW(h_process, image_file_name.as_mut_ptr(), image_file_name.len() as u32) as usize;

                OsString::from_wide(&image_file_name[0..name_length])
            };

            let path = PathBuf::from(&process_name);

            if let Some(executable) = path.file_name() {
                if executable == executable_os {
                    return true;
                }
            }
        }
    }
    
    false
}

fn main() -> Result<(), Error> {
    // Our current best guess as to the state of the game
    let mut current_game_state = GameState::default();

    // possible_game_state history
    let mut possible_history = [GameState::default(); 3];
    let mut possible_history_index = 0;

    loop {
        // Go to sleep for a little bit
        std::thread::sleep(std::time::Duration::from_secs(1));

        // Check to make sure CS:GO is running
        if !check_process_exists("csgo.exe") {
            // If it isn't running, don't do anything and wait some more
            continue;
        }

        println!("cs is running, screenshotting");

        // Take a screenshot and try to deduce the game state
        possible_history_index = (possible_history_index + 1) % possible_history.len();
        possible_history[possible_history_index] = process_game_info()?;

        // If the game state has not changed in the past 3 iterations, we are fairly
        // certain that the recorded game state is correct, so update it.
        if possible_history[0] == possible_history[1] && possible_history[0] == possible_history[2] {
            current_game_state = possible_history[0];
        }

        println!("{:?}", current_game_state);
    }

    Ok(())
}
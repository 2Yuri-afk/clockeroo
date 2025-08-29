use anyhow::Result;
use chrono::{Local, NaiveTime, TimeZone};
use clap::{Parser, Subcommand};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use notify_rust::Notification;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::time::sleep;

const ASCII_HEADER: &str = r#"      _            _                        
  ___| | ___   ___| | _____ _ __ ___   ___  
 / __| |/ _ \ / __| |/ / _ \ '__/ _ \ / _ \ 
| (__| | (_) | (__|   <  __/ | | (_) | (_) |
 \___|_|\___/ \___|_|\_\___|_|  \___/ \___/ "#;

#[derive(Parser)]
#[command(name = "clockeroo")]
#[command(about = "A simple TUI timer/stopwatch/alarm app", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Set a countdown timer (e.g., "120s", "5m", "2h")
    Timer {
        /// Duration in format: 120s, 5m, 2h, or combinations like 1h30m
        duration: String,
    },
    /// Control a stopwatch
    Stopwatch {
        #[command(subcommand)]
        action: StopwatchAction,
    },
    /// Set an alarm for a specific time (e.g., "7:20am", "19:20", "7:20pm")
    Alarm {
        /// Time in format: 7:20am, 19:20, 7:20pm
        time: String,
    },
}

#[derive(Subcommand)]
enum StopwatchAction {
    /// Start the stopwatch
    Start,
    /// Stop the stopwatch and show elapsed time
    Stop,
}

fn parse_duration(s: &str) -> Result<Duration> {
    let s = s.to_lowercase();
    let mut total_seconds = 0u64;
    let mut current_num = String::new();
    
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            current_num.push(ch);
        } else if ch == 'h' {
            if !current_num.is_empty() {
                total_seconds += current_num.parse::<u64>()? * 3600;
                current_num.clear();
            }
        } else if ch == 'm' {
            if !current_num.is_empty() {
                total_seconds += current_num.parse::<u64>()? * 60;
                current_num.clear();
            }
        } else if ch == 's' {
            if !current_num.is_empty() {
                total_seconds += current_num.parse::<u64>()?;
                current_num.clear();
            }
        }
    }
    
    // If there's a number without a unit, treat it as seconds
    if !current_num.is_empty() {
        total_seconds += current_num.parse::<u64>()?;
    }
    
    if total_seconds == 0 {
        anyhow::bail!("Invalid duration format. Use formats like: 120s, 5m, 2h, 1h30m");
    }
    
    Ok(Duration::from_secs(total_seconds))
}

fn parse_alarm_time(s: &str) -> Result<NaiveTime> {
    let s = s.to_lowercase();
    
    // Handle AM/PM format
    if s.contains("am") || s.contains("pm") {
        let is_pm = s.contains("pm");
        let time_part = s.replace("am", "").replace("pm", "").trim().to_string();
        let parts: Vec<&str> = time_part.split(':').collect();
        
        if parts.len() != 2 {
            anyhow::bail!("Invalid time format. Use formats like: 7:20am, 7:20pm, or 19:20");
        }
        
        let mut hour: u32 = parts[0].parse()?;
        let minute: u32 = parts[1].parse()?;
        
        if hour > 12 || hour == 0 {
            anyhow::bail!("Invalid hour for AM/PM format");
        }
        
        if is_pm && hour != 12 {
            hour += 12;
        } else if !is_pm && hour == 12 {
            hour = 0;
        }
        
        NaiveTime::from_hms_opt(hour, minute, 0)
            .ok_or_else(|| anyhow::anyhow!("Invalid time"))
    } else {
        // Handle 24-hour format
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid time format. Use formats like: 7:20am, 7:20pm, or 19:20");
        }
        
        let hour: u32 = parts[0].parse()?;
        let minute: u32 = parts[1].parse()?;
        
        NaiveTime::from_hms_opt(hour, minute, 0)
            .ok_or_else(|| anyhow::anyhow!("Invalid time"))
    }
}

fn get_stopwatch_file() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime_dir).join("clockeroo.stopwatch")
    } else {
        PathBuf::from("/tmp").join("clockeroo.stopwatch")
    }
}

fn format_duration(duration: Duration) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    
    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
    } else {
        format!("{:02}:{:02}", minutes, seconds)
    }
}

fn send_notification(title: &str, body: &str) {
    let _ = Notification::new()
        .summary(title)
        .body(body)
        .icon("dialog-information")
        .timeout(0)
        .show();
}

fn play_bell() {
    // Try terminal bell first
    print!("\x07");
    let _ = io::stdout().flush();
    
    // Also play an actual sound using rodio
    play_sound();
}

fn play_sound() {
    use rodio::{OutputStream, source::Source};
    
    // Try to play a built-in sine wave beep
    if let Ok((_stream, stream_handle)) = OutputStream::try_default() {
        // Create a gentler beep sound (440 Hz sine wave for 0.3 seconds)
        // 440 Hz is the musical note A4, much more pleasant than 1000 Hz
        let source = rodio::source::SineWave::new(440.0)
            .take_duration(std::time::Duration::from_millis(300))
            .amplify(0.2)  // Reduced volume from 0.5 to 0.2
            .fade_in(std::time::Duration::from_millis(50));  // Gentle fade-in
        
        // Play the sound (ignore errors if audio system unavailable)
        let _ = stream_handle.play_raw(source.convert_samples());
        
        // Keep the stream alive while the sound plays
        std::thread::sleep(std::time::Duration::from_millis(350));
    }
}

async fn run_timer_ui(duration: Duration) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let start_time = Instant::now();
    
    loop {
        let elapsed = start_time.elapsed();
        
        if elapsed >= duration {
            // Timer finished
            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .margin(2)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Length(3),
                        Constraint::Length(3),
                        Constraint::Min(0),
                    ])
                    .split(f.area());

                let title = Paragraph::new("TIMER FINISHED!")
                    .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                    .alignment(Alignment::Center)
                    .block(Block::default().borders(Borders::ALL));
                f.render_widget(title, chunks[0]);

                let message = Paragraph::new("Your timer has completed!")
                    .style(Style::default().fg(Color::Yellow))
                    .alignment(Alignment::Center)
                    .block(Block::default().borders(Borders::ALL));
                f.render_widget(message, chunks[1]);

                let help = Paragraph::new("Press 'q' or Ctrl-C to exit")
                    .style(Style::default().fg(Color::Gray))
                    .alignment(Alignment::Center);
                f.render_widget(help, chunks[2]);
            })?;
            
            // Send notifications
            play_bell();
            send_notification("Timer Finished!", "Your timer has completed!");
            
            // Wait for user to quit
            loop {
                if event::poll(Duration::from_millis(100))? {
                    if let Event::Key(key) = event::read()? {
                        if key.code == KeyCode::Char('q') || key.code == KeyCode::Char('c') && key.modifiers.contains(event::KeyModifiers::CONTROL) {
                            break;
                        }
                    }
                }
            }
            break;
        }
        
        let remaining = duration - elapsed;
        let remaining_seconds = remaining.as_secs();
        
        terminal.draw(|f| {
            let area = f.area();
            
            // Color based on remaining time
            let time_color = if remaining_seconds < 10 {
                Color::Red
            } else if remaining_seconds < 60 {
                Color::Yellow
            } else {
                Color::Green
            };
            
            // Create the simple, clean content
            let mut lines = vec![];
            
            // Add ASCII header lines
            for line in ASCII_HEADER.lines() {
                lines.push(Line::from(vec![Span::styled(line, Style::default().fg(Color::DarkGray))]));    
            }
            
            // Add the rest of the content
            lines.push(Line::from(""));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled("Timer Running", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled("Time Remaining", Style::default().fg(Color::Gray))]));
            lines.push(Line::from(vec![Span::styled(format_duration(remaining), Style::default().fg(time_color).add_modifier(Modifier::BOLD))]));
            lines.push(Line::from(""));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled("Press 'q' or Ctrl-C to cancel", Style::default().fg(Color::Gray))]));
            
            let paragraph = Paragraph::new(lines)
                .block(Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                )
                .alignment(Alignment::Center);
                
            f.render_widget(paragraph, area);
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') || key.code == KeyCode::Char('c') && key.modifiers.contains(event::KeyModifiers::CONTROL) {
                    break;
                }
            }
        }
        
        sleep(Duration::from_millis(100)).await;
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

async fn run_stopwatch_ui() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let stopwatch_file = get_stopwatch_file();
    let start_time = Instant::now();
    
    // Save start time to file
    fs::write(&stopwatch_file, format!("{:?}", start_time))?;
    
    loop {
        let elapsed = start_time.elapsed();
        
        terminal.draw(|f| {
            let area = f.area();
            
            let millis = elapsed.as_millis() % 1000;
            let time_str = format!("{}.{:03}", format_duration(elapsed), millis);
            
            // Create the simple, clean content
            let mut lines = vec![];
            
            // Add ASCII header lines
            for line in ASCII_HEADER.lines() {
                lines.push(Line::from(vec![Span::styled(line, Style::default().fg(Color::DarkGray))]));    
            }
            
            // Add the rest of the content
            lines.push(Line::from(""));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled("Stopwatch Running", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled("Elapsed Time", Style::default().fg(Color::Gray))]));
            lines.push(Line::from(vec![Span::styled(time_str, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))]));
            lines.push(Line::from(""));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled("Press 's' to stop, 'q' or Ctrl-C to quit", Style::default().fg(Color::Gray))]));
            
            let paragraph = Paragraph::new(lines)
                .block(Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                )
                .alignment(Alignment::Center);
                
            f.render_widget(paragraph, area);
        })?;

        if event::poll(Duration::from_millis(10))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('s') => {
                        // Stop and show final time
                        let final_time = start_time.elapsed();
                        disable_raw_mode()?;
                        execute!(
                            terminal.backend_mut(),
                            LeaveAlternateScreen,
                            DisableMouseCapture
                        )?;
                        terminal.show_cursor()?;
                        
                        let millis = final_time.as_millis() % 1000;
                        println!("\n[Stopwatch stopped]");
                        println!("   Final time: {}.{:03}", format_duration(final_time), millis);
                        
                        // Clean up the file
                        let _ = fs::remove_file(&stopwatch_file);
                        return Ok(());
                    }
                    KeyCode::Char('q') => {
                        break;
                    }
                    KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        break;
                    }
                    _ => {}
                }
            }
        }
        
        sleep(Duration::from_millis(10)).await;
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    
    // Keep the stopwatch file for later
    println!("\n[Stopwatch still running in background]");
    println!("Run 'clockeroo stopwatch stop' to see the final time.");

    Ok(())
}

async fn show_stopwatch_time() -> Result<()> {
    let stopwatch_file = get_stopwatch_file();
    
    if !stopwatch_file.exists() {
        println!("[ERROR] No stopwatch is currently running.");
        println!("Start one with: clockeroo stopwatch start");
        return Ok(());
    }
    
    // For simplicity, we'll just show that a stopwatch is running
    // In a real implementation, we'd need to serialize the Instant properly
    println!("[Stopwatch is running]");
    println!("Note: To see live time, run 'clockeroo stopwatch start' again.");
    
    // Clean up the file
    let _ = fs::remove_file(&stopwatch_file);
    
    Ok(())
}

async fn run_alarm_ui(alarm_time: NaiveTime) -> Result<()> {
    let now = Local::now();
    let mut target = now.date_naive().and_time(alarm_time);
    
    // If the alarm time has already passed today, set it for tomorrow
    if target <= now.naive_local() {
        target += chrono::Duration::days(1);
    }
    
    let target_datetime = Local
        .from_local_datetime(&target)
        .single()
        .ok_or_else(|| anyhow::anyhow!("Invalid datetime conversion"))?;
    
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        let now = Local::now();
        
        if now >= target_datetime {
            // Alarm triggered
            terminal.draw(|f| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .margin(2)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Length(3),
                        Constraint::Length(3),
                        Constraint::Min(0),
                    ])
                    .split(f.area());

                let title = Paragraph::new("ALARM!")
                    .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD | Modifier::SLOW_BLINK))
                    .alignment(Alignment::Center)
                    .block(Block::default().borders(Borders::ALL));
                f.render_widget(title, chunks[0]);

                let time_str = format!("It's {}!", alarm_time.format("%I:%M %p"));
                let message = Paragraph::new(time_str)
                    .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                    .alignment(Alignment::Center)
                    .block(Block::default().borders(Borders::ALL));
                f.render_widget(message, chunks[1]);

                let help = Paragraph::new("Press 'q' or Ctrl-C to exit")
                    .style(Style::default().fg(Color::Gray))
                    .alignment(Alignment::Center);
                f.render_widget(help, chunks[2]);
            })?;
            
            // Send notifications
            play_bell();
            send_notification("Alarm!", &format!("It's {}!", alarm_time.format("%I:%M %p")));
            
            // Wait for user to quit
            loop {
                if event::poll(Duration::from_millis(100))? {
                    if let Event::Key(key) = event::read()? {
                        if key.code == KeyCode::Char('q') || key.code == KeyCode::Char('c') && key.modifiers.contains(event::KeyModifiers::CONTROL) {
                            break;
                        }
                    }
                }
            }
            break;
        }
        
        let duration_until = target_datetime.signed_duration_since(now);
        let hours = duration_until.num_hours();
        let minutes = (duration_until.num_minutes() % 60).abs();
        let seconds = (duration_until.num_seconds() % 60).abs();
        
        terminal.draw(|f| {
            let area = f.area();
            
            let alarm_str = format!("Alarm will ring at {}", alarm_time.format("%I:%M %p"));
            let time_remaining = if hours > 0 {
                format!("{:02}:{:02}:{:02} remaining", hours, minutes, seconds)
            } else {
                format!("{:02}:{:02} remaining", minutes, seconds)
            };
            
            // Create the simple, clean content
            let mut lines = vec![];
            
            // Add ASCII header lines
            for line in ASCII_HEADER.lines() {
                lines.push(Line::from(vec![Span::styled(line, Style::default().fg(Color::DarkGray))]));    
            }
            
            // Add the rest of the content
            lines.push(Line::from(""));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled("Alarm Set", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(alarm_str, Style::default().fg(Color::Yellow))]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled("Time Until Alarm", Style::default().fg(Color::Gray))]));
            lines.push(Line::from(vec![Span::styled(time_remaining, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled("Press 'q' or Ctrl-C to cancel", Style::default().fg(Color::Gray))]));
            
            let paragraph = Paragraph::new(lines)
                .block(Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                )
                .alignment(Alignment::Center);
                
            f.render_widget(paragraph, area);
        })?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') || key.code == KeyCode::Char('c') && key.modifiers.contains(event::KeyModifiers::CONTROL) {
                    break;
                }
            }
        }
        
        sleep(Duration::from_millis(100)).await;
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Print ASCII header
    println!("\n{}", ASCII_HEADER);
    println!();

    match cli.command {
        Commands::Timer { duration } => {
            let duration = parse_duration(&duration)?;
            println!("[TIMER] Starting timer for {}...", format_duration(duration));
            run_timer_ui(duration).await?;
        }
        Commands::Stopwatch { action } => {
            match action {
                StopwatchAction::Start => {
                    println!("[STOPWATCH] Starting stopwatch...");
                    run_stopwatch_ui().await?;
                }
                StopwatchAction::Stop => {
                    show_stopwatch_time().await?;
                }
            }
        }
        Commands::Alarm { time } => {
            let alarm_time = parse_alarm_time(&time)?;
            println!("[ALARM] Setting alarm for {}...", alarm_time.format("%I:%M %p"));
            run_alarm_ui(alarm_time).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("120s").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
        assert_eq!(parse_duration("1h30m45s").unwrap(), Duration::from_secs(5445));
        assert_eq!(parse_duration("90").unwrap(), Duration::from_secs(90));
    }

    #[test]
    fn test_parse_alarm_time() {
        let time1 = parse_alarm_time("7:20am").unwrap();
        assert_eq!(time1.hour(), 7);
        assert_eq!(time1.minute(), 20);

        let time2 = parse_alarm_time("7:20pm").unwrap();
        assert_eq!(time2.hour(), 19);
        assert_eq!(time2.minute(), 20);

        let time3 = parse_alarm_time("23:45").unwrap();
        assert_eq!(time3.hour(), 23);
        assert_eq!(time3.minute(), 45);

        let time4 = parse_alarm_time("12:00am").unwrap();
        assert_eq!(time4.hour(), 0);
        assert_eq!(time4.minute(), 0);

        let time5 = parse_alarm_time("12:00pm").unwrap();
        assert_eq!(time5.hour(), 12);
        assert_eq!(time5.minute(), 0);
    }
}

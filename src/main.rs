use chrono::prelude::*;
use crossterm::{
    event::{self, Event as CEvent, KeyCode, KeyEvent},
    terminal,
};
use rand::{distributions::Alphanumeric, prelude::*};
use serde::{Deserialize, Serialize};
use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use std::{fs, io::Stdout, sync::mpsc::Receiver};
use thiserror::Error;
use tui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{
        Block, BorderType, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table, Tabs,
    },
    Terminal,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    terminal::enable_raw_mode().expect("can run in raw mode");

    let (tx, mut rx) = mpsc::channel();
    thread::spawn(move || accept_user_input(Duration::from_millis(200), tx));
    let mut app_state = AppState::default();
    let mut terminal = create_terminal()?;

    loop {
        terminal.draw(|rect| {
            draw(
                rect,
                &app_state.menu_titles,
                app_state.active_menu_item,
                &mut app_state.pet_list_state,
            );
        })?;

        let input_response = handle_user_input(
            &mut rx,
            &mut terminal,
            &mut app_state.active_menu_item,
            &mut app_state.pet_list_state,
        )?;
        if input_response == ResponseToUserInput::Stop {
            break;
        }
    }

    Ok(())
}

const DB_PATH: &str = "./data/db.json";

#[derive(Error, Debug)]
pub enum Error {
    #[error("error reading the DB file: {0}")]
    ReadDBError(#[from] io::Error),
    #[error("error parsing the DB file: {0}")]
    ParseDBError(#[from] serde_json::Error),
}

enum Event<I> {
    Input(I),
    Tick,
}

#[derive(Serialize, Deserialize, Clone)]
struct Pet {
    id: usize,
    name: String,
    category: String,
    age: usize,
    created_at: DateTime<Utc>,
}

#[derive(Copy, Clone, Debug)]
enum MenuItem {
    Home,
    Pets,
}

impl From<MenuItem> for usize {
    fn from(input: MenuItem) -> usize {
        match input {
            MenuItem::Home => 0,
            MenuItem::Pets => 1,
        }
    }
}

struct AppState<'a> {
    menu_titles: Vec<&'a str>,
    active_menu_item: MenuItem,
    pet_list_state: ListState,
}

impl Default for AppState<'_> {
    fn default() -> Self {
        let mut pet_list_state = ListState::default();
        pet_list_state.select(Some(0));
        Self {
            menu_titles: vec!["Home", "Pets", "Add", "Delete", "Quit"],
            active_menu_item: MenuItem::Home,
            pet_list_state,
        }
    }
}

#[derive(PartialEq)]
enum ResponseToUserInput {
    Continue,
    Stop,
}

fn handle_user_input(
    rx: &mut Receiver<Event<KeyEvent>>,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    active_menu_item: &mut MenuItem,
    pet_list_state: &mut ListState,
) -> Result<ResponseToUserInput, Box<dyn std::error::Error>> {
    let Event::Input(event) = rx.recv()? else {
        return Ok(ResponseToUserInput::Continue);
    };
    match event.code {
        KeyCode::Char('q') => {
            terminal::disable_raw_mode()?;
            terminal.show_cursor()?;
            return Ok(ResponseToUserInput::Stop);
        }
        KeyCode::Char('h') => *active_menu_item = MenuItem::Home,
        KeyCode::Char('p') => *active_menu_item = MenuItem::Pets,
        KeyCode::Char('a') => {
            add_random_pet_to_db().expect("can add new random pet");
        }
        KeyCode::Char('d') => {
            remove_pet_at_index(pet_list_state).expect("can remove pet");
        }
        KeyCode::Char('j') => {
            if let Some(selected) = pet_list_state.selected() {
                let amount_pets = read_db().expect("can fetch pet list").len();
                if selected >= amount_pets - 1 {
                    pet_list_state.select(Some(0));
                } else {
                    pet_list_state.select(Some(selected + 1));
                }
            }
        }
        KeyCode::Char('k') => {
            if let Some(selected) = pet_list_state.selected() {
                let amount_pets = read_db().expect("can fetch pet list").len();
                if selected > 0 {
                    pet_list_state.select(Some(selected - 1));
                } else {
                    pet_list_state.select(Some(amount_pets - 1));
                }
            }
        }
        _ => {}
    }
    Ok(ResponseToUserInput::Continue)
}

fn create_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>, Box<dyn std::error::Error>> {
    let stdout = io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

fn accept_user_input(tick_rate: Duration, tx: mpsc::Sender<Event<KeyEvent>>) {
    let mut last_tick = Instant::now();
    loop {
        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout).expect("poll works") {
            if let CEvent::Key(key) = event::read().expect("can read events") {
                tx.send(Event::Input(key)).expect("can send events");
            }
        }

        if last_tick.elapsed() >= tick_rate && tx.send(Event::Tick).is_ok() {
            last_tick = Instant::now();
        }
    }
}

fn draw(
    total_drawing_rect: &mut tui::Frame<CrosstermBackend<io::Stdout>>,
    menu_titles: &[&str],
    active_menu_item: MenuItem,
    pet_list_state: &mut ListState,
) {
    let app_rects = create_app_rects(total_drawing_rect.size());
    let copyright = create_copyright_paragraph();
    let tabs = create_tabs(create_menu(menu_titles), active_menu_item);
    total_drawing_rect.render_widget(tabs, app_rects.menu);
    render_selected_widget(
        active_menu_item,
        total_drawing_rect,
        &app_rects,
        pet_list_state,
    );
    total_drawing_rect.render_widget(copyright, app_rects.copyright);
}

fn render_selected_widget(
    active_menu_item: MenuItem,
    rect: &mut tui::Frame<CrosstermBackend<io::Stdout>>,
    app_rects: &AppRects,
    pet_list_state: &mut ListState,
) {
    match active_menu_item {
        MenuItem::Home => rect.render_widget(render_home(), app_rects.main_widget),
        MenuItem::Pets => {
            let pet_rects = create_pet_rects(&app_rects.main_widget);
            let (left, right) = create_pet_widgets(pet_list_state);
            rect.render_stateful_widget(left, pet_rects.names, pet_list_state);
            rect.render_widget(right, pet_rects.details);
        }
    }
}

struct PetRects {
    names: Rect,
    details: Rect,
}

fn create_pet_rects(parent_rect: &Rect) -> PetRects {
    let pet_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(20), Constraint::Percentage(80)].as_ref())
        .split(*parent_rect);
    PetRects {
        names: pet_rects[0],
        details: pet_rects[1],
    }
}

fn create_tabs<'a>(menu: Vec<Spans<'a>>, active_menu_item: MenuItem) -> Tabs<'a> {
    Tabs::new(menu)
        .select(active_menu_item.into())
        .block(Block::default().title("Menu").borders(Borders::ALL))
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().fg(Color::Yellow))
        .divider(Span::raw("|"))
}

struct AppRects {
    menu: Rect,
    main_widget: Rect,
    copyright: Rect,
}

fn create_app_rects(total_drawing_rect: Rect) -> AppRects {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(vec![
            Constraint::Length(3),
            Constraint::Min(2),
            Constraint::Length(3),
        ])
        .split(total_drawing_rect);
    AppRects {
        menu: areas[0],
        main_widget: areas[1],
        copyright: areas[2],
    }
}

fn create_menu<'a>(menu_titles: &[&'a str]) -> Vec<Spans<'a>> {
    menu_titles
        .iter()
        .map(|t| {
            let (first, rest) = t.split_at(1);
            Spans::from(vec![
                Span::styled(
                    first,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::UNDERLINED),
                ),
                Span::styled(rest, Style::default().fg(Color::White)),
            ])
        })
        .collect()
}

fn create_copyright_paragraph<'a>() -> Paragraph<'a> {
    Paragraph::new("pet-CLI 2020 - all rights reserved")
        .style(Style::default().fg(Color::LightCyan))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .style(Style::default().fg(Color::White))
                .title("Copyright")
                .border_type(BorderType::Plain),
        )
}

fn render_home<'a>() -> Paragraph<'a> {
    let home = Paragraph::new(vec![
        Spans::from(vec![Span::raw("")]),
        Spans::from(vec![Span::raw("Welcome")]),
        Spans::from(vec![Span::raw("")]),
        Spans::from(vec![Span::raw("to")]),
        Spans::from(vec![Span::raw("")]),
        Spans::from(vec![Span::styled(
            "pet-CLI",
            Style::default().fg(Color::LightBlue),
        )]),
        Spans::from(vec![Span::raw("")]),
        Spans::from(vec![Span::raw("Press 'p' to access pets, 'a' to add random new pets and 'd' to delete the currently selected pet.")]),
    ])
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White))
            .title("Home")
            .border_type(BorderType::Plain),
    );
    home
}

fn create_pet_widgets<'a>(pet_list_state: &ListState) -> (List<'a>, Table<'a>) {
    let pets = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().fg(Color::White))
        .title("Pets")
        .border_type(BorderType::Plain);

    let pet_list = read_db().expect("can fetch pet list");
    let items: Vec<_> = pet_list
        .iter()
        .map(|pet| {
            ListItem::new(Spans::from(vec![Span::styled(
                pet.name.clone(),
                Style::default(),
            )]))
        })
        .collect();

    let selected_pet = pet_list
        .get(
            pet_list_state
                .selected()
                .expect("there is always a selected pet"),
        )
        .expect("exists")
        .clone();

    let list = List::new(items).block(pets).highlight_style(
        Style::default()
            .bg(Color::Yellow)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD),
    );

    let pet_detail = Table::new(vec![Row::new(vec![
        Cell::from(Span::raw(selected_pet.id.to_string())),
        Cell::from(Span::raw(selected_pet.name)),
        Cell::from(Span::raw(selected_pet.category)),
        Cell::from(Span::raw(selected_pet.age.to_string())),
        Cell::from(Span::raw(selected_pet.created_at.to_string())),
    ])])
    .header(Row::new(vec![
        Cell::from(Span::styled(
            "ID",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Name",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Category",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Age",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Cell::from(Span::styled(
            "Created At",
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White))
            .title("Detail")
            .border_type(BorderType::Plain),
    )
    .widths(&[
        Constraint::Percentage(5),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(5),
        Constraint::Percentage(20),
    ]);

    (list, pet_detail)
}

fn read_db() -> Result<Vec<Pet>, Error> {
    let db_content = fs::read_to_string(DB_PATH)?;
    let parsed: Vec<Pet> = serde_json::from_str(&db_content)?;
    Ok(parsed)
}

fn add_random_pet_to_db() -> Result<Vec<Pet>, Error> {
    let mut rng = rand::thread_rng();
    let db_content = fs::read_to_string(DB_PATH)?;
    let mut parsed: Vec<Pet> = serde_json::from_str(&db_content)?;
    let catsdogs = match rng.gen_range(0, 1) {
        0 => "cats",
        _ => "dogs",
    };

    let random_pet = Pet {
        id: rng.gen_range(0, 9999999),
        name: rng.sample_iter(Alphanumeric).take(10).collect(),
        category: catsdogs.to_owned(),
        age: rng.gen_range(1, 15),
        created_at: Utc::now(),
    };

    parsed.push(random_pet);
    fs::write(DB_PATH, serde_json::to_vec(&parsed)?)?;
    Ok(parsed)
}

fn remove_pet_at_index(pet_list_state: &mut ListState) -> Result<(), Error> {
    let Some(selected) = pet_list_state.selected() else {
        return Ok(());
    };
    let db_content = fs::read_to_string(DB_PATH)?;
    let mut parsed: Vec<Pet> = serde_json::from_str(&db_content)?;
    parsed.remove(selected);
    fs::write(DB_PATH, serde_json::to_vec(&parsed)?)?;
    if selected > 0 {
        pet_list_state.select(Some(selected - 1));
    } else {
        pet_list_state.select(Some(0));
    }
    Ok(())
}

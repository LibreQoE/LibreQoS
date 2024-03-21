use ratatui::{style::{Color, Style}, text::Text, widgets::{Block, Borders, Cell, Row, Table}};

/// A helper for building Ratatui tables
pub struct TableHelper<const N_COLS: usize> {
    headers: [String; N_COLS],
    rows: Vec<[String; N_COLS]>,
}

impl <const N: usize> TableHelper<N> {
    pub fn new<S: ToString>(raw_headers: [S; N]) -> Self {
        const ARRAY_REPEAT_VALUE: String = String::new();
        let mut headers = [ARRAY_REPEAT_VALUE; N];
        for i in 0..N {
            headers[i] = raw_headers[i].to_string();
        }
        
        let headers = headers.try_into().unwrap();
        Self {
            headers,
            rows: Vec::new(),
        }
    }

    pub fn add_row(&mut self, row: [String; N]) {
        self.rows.push(row);
    }

    pub fn to_table(&self) -> Table {
        let header_cells: Vec<_> = self.headers.
            iter()
            .map(|h| Cell::from(Text::from(h.clone())))
            .collect();

        let rows: Vec<_> = self.rows
            .iter()
            .map(|row| {
                let cells = row.iter()
                    .map(|cell| Cell::from(Text::from(cell.clone())))
                    .collect::<Vec<Cell>>();
                Row::new(cells)
            })
            .collect();

        let mut widths = [0u16; N];
        for (i, heading) in self.headers.iter().enumerate() {
            widths[i] = (heading.len() + 2) as u16;
        }
        for row in self.rows.iter() {
            for (j, cell) in row.iter().enumerate() {
                if cell.len() + 2 > widths[j] as usize {
                    widths[j] = (cell.len() + 2) as u16;
                }
            }
        }

        Table::new(rows, widths)
            .header(Row::new(header_cells).style(Style::default().fg(Color::White).bg(Color::Blue)))
    }

    pub fn to_block(&self) -> Table {
        let block = Block::default()
            //.title("Top Downloaders")
            .borders(Borders::NONE)
            .style(Style::default().fg(Color::Green));
        self.to_table().block(block)
    }
}
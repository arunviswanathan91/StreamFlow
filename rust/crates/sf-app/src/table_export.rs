//! A statistics table as spreadsheet rows (Excel) and as paginated PDF pages.

use crate::figure::{text_width, Align, Figure, Item, Stroke, Text, VAlign, BLACK};
use crate::office::Cell;
use crate::stats_table::{pop_label, Metric, Table};

fn fmt_value(m: Metric, v: f64) -> String {
    match m {
        Metric::Count => format!("{v:.0}"),
        Metric::PctParent | Metric::PctTotal | Metric::Cv | Metric::RobustCv => format!("{v:.2}"),
        _ => format!("{v:.1}"),
    }
}

/// Header row, then one row per sample and population. Undefined values are blank cells, never zero.
pub fn table_rows(t: &Table) -> Vec<Vec<Cell>> {
    let mut head = vec![Cell::Text("Sample".into()), Cell::Text("Population".into())];
    head.extend(t.metrics.iter().map(|m| Cell::Text(match (&t.channel, m.needs_channel()) {
        (Some(c), true) => format!("{} {c}", m.label()),
        _ => m.label(),
    })));
    let mut rows = vec![head];
    for r in &t.rows {
        let mut row = vec![Cell::Text(r.sample.clone()), Cell::Text(pop_label(&r.population))];
        row.extend(r.values.iter().map(|v| v.map_or(Cell::Empty, Cell::Num)));
        rows.push(row);
    }
    rows
}

/// Landscape Letter pages, the header repeated on each, with the table's notes on the last page.
pub fn table_pages(t: &Table, title: &str) -> Vec<Figure> {
    const W: f32 = 792.0;
    const H: f32 = 612.0;
    const M: f32 = 36.0;
    const ROW: f32 = 13.0;
    const SIZE: f32 = 7.5;
    let rows = table_rows(t);
    let text_of = |c: &Cell, m: Option<Metric>| match c {
        Cell::Text(s) => s.clone(),
        Cell::Num(v) => m.map_or(format!("{v}"), |m| fmt_value(m, *v)),
        Cell::Empty => String::new(),
    };
    let metric_of = |col: usize| if col >= 2 { t.metrics.get(col - 2).copied() } else { None };
    // column widths from the widest cell, capped; shrunk together when the table is too wide
    let ncols = rows[0].len();
    let mut widths: Vec<f32> = (0..ncols).map(|c| rows.iter().map(|r| text_width(&text_of(&r[c], metric_of(c)), SIZE, c == 0 && false)).fold(0.0, f32::max).min(240.0) + 10.0).collect();
    let total: f32 = widths.iter().sum();
    if total > W - 2.0 * M {
        let k = (W - 2.0 * M) / total;
        widths.iter_mut().for_each(|w| *w *= k);
    }
    let per_page = (((H - 2.0 * M - 30.0) / ROW) as usize).saturating_sub(1).max(1);
    let body = &rows[1..];
    let n_pages = body.len().div_ceil(per_page).max(1);
    let mut pages = Vec::new();
    for p in 0..n_pages {
        let mut f = Figure::new(W, H);
        f.push(Item::Text(Text::new(M, M - 8.0, title, 11.0).bold().align(Align::Left, VAlign::Bottom)));
        f.push(Item::Text(Text::new(W - M, M - 8.0, format!("Page {} of {n_pages}", p + 1), 8.0).align(Align::Right, VAlign::Bottom).color([110, 110, 110, 255])));
        let mut y = M + 8.0;
        let draw_row = |f: &mut Figure, row: &[Cell], y: f32, head: bool| {
            let mut x = M;
            for (c, cell) in row.iter().enumerate() {
                let mut t = Text::new(if c >= 2 { x + widths[c] - 5.0 } else { x + 3.0 }, y + ROW / 2.0, text_of(cell, metric_of(c)), SIZE).align(if c >= 2 { Align::Right } else { Align::Left }, VAlign::Middle);
                if head {
                    t = t.bold();
                }
                // long text is cut to its column rather than overprinting the next one
                let avail = widths[c] - 8.0;
                while text_width(&t.text, SIZE, head) > avail && t.text.chars().count() > 1 {
                    t.text.pop();
                }
                f.push(Item::Text(t));
                x += widths[c];
            }
            f.push(Item::Line { a: (M, y + ROW), b: (M + widths.iter().sum::<f32>(), y + ROW), stroke: Stroke::new(if head { BLACK } else { [215, 215, 215, 255] }, if head { 0.8 } else { 0.4 }) });
        };
        draw_row(&mut f, &rows[0], y, true);
        y += ROW;
        for row in body.iter().skip(p * per_page).take(per_page) {
            draw_row(&mut f, row, y, false);
            y += ROW;
        }
        if p + 1 == n_pages {
            for n in &t.notes {
                y += 4.0;
                f.push(Item::Text(Text::new(M, y + 8.0, n.clone(), 7.0).color([120, 80, 0, 255]).align(Align::Left, VAlign::Middle)));
                y += 9.0;
            }
        }
        pages.push(f);
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats_table::Row;

    fn table(n: usize) -> Table {
        Table {
            metrics: vec![Metric::Count, Metric::PctParent, Metric::Median],
            channel: Some("FITC-A".into()),
            rows: (0..n).map(|i| Row { sample: format!("sample {i}.fcs"), population: if i % 3 == 0 { vec![] } else { vec!["Lymphocytes".into(), "CD3+".into()] }, values: vec![Some(1000.0 + i as f64), Some(45.678), if i % 5 == 0 { None } else { Some(12345.6) }] }).collect(),
            notes: vec!["geometric mean ignored 3 events".into()],
        }
    }

    #[test]
    fn rows_have_a_header_and_blank_cells_for_undefined_values() {
        let r = table_rows(&table(6));
        assert_eq!(r.len(), 7);
        assert_eq!(r[0], vec![Cell::Text("Sample".into()), Cell::Text("Population".into()), Cell::Text("Count".into()), Cell::Text("% of parent".into()), Cell::Text("Median FITC-A".into())]);
        assert_eq!(r[1][1], Cell::Text("All Events".into()));
        assert_eq!(r[1][4], Cell::Empty, "undefined is blank, never zero");
        assert_eq!(r[2][2], Cell::Num(1001.0));
    }

    #[test]
    fn pages_hold_every_row_once_repeat_the_header_and_form_a_valid_pdf() {
        let t = table(120);
        let pages = table_pages(&t, "Group: Controls");
        assert!(pages.len() >= 3);
        let all: Vec<String> = pages.iter().flat_map(|p| p.texts()).collect();
        for i in [0usize, 59, 119] {
            assert_eq!(all.iter().filter(|s| **s == format!("sample {i}.fcs")).count(), 1, "row {i} appears exactly once");
        }
        assert_eq!(all.iter().filter(|s| s.as_str() == "Median FITC-A").count(), pages.len(), "the header is on every page");
        assert!(all.iter().any(|s| s.contains("Page 2 of")));
        assert!(pages.last().unwrap().texts().iter().any(|s| s.contains("geometric mean ignored")), "notes are printed on the last page");
        assert_eq!(crate::figure::validate_pdf(&crate::figure::to_pdf(&pages, "t")), Ok(pages.len()));
        // an empty table still gives one page with the header
        assert_eq!(table_pages(&table(0), "x").len(), 1);
        // numbers are formatted by kind
        assert!(all.iter().any(|s| s == "45.68") && all.iter().any(|s| s == "12345.6") && all.iter().any(|s| s == "1000"));
    }
}

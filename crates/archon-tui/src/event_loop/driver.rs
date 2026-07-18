//! Scheduler, source selection, and bounded TUI-event draining for the render loop.

use std::io;
use std::time::Duration;

use crossterm::event::Event;
use futures_util::{Stream, StreamExt};

use crate::app::{App, TuiEvent};
use crate::event_channel::TuiEventReceiver;

/// Cadence for idle and active animation frames.
pub(super) const IDLE_TICK_CADENCE: Duration = Duration::from_millis(250);
const ACTIVE_TICK_CADENCE: Duration = Duration::from_millis(80);
/// Maximum agent-to-TUI events applied before returning to select/draw.
const MAX_TUI_EVENTS_PER_FRAME: usize = 64;

/// Owns the animation deadline across render-loop iterations.
///
/// Replacing the interval only when the requested cadence changes prevents a
/// steady stream of input from continually pushing the next deadline forward.
/// Skipping missed ticks also avoids a burst of catch-up animation frames.
pub(super) struct TickScheduler {
    cadence: Duration,
    interval: tokio::time::Interval,
}

impl TickScheduler {
    pub(super) fn new(cadence: Duration) -> Self {
        Self {
            cadence,
            interval: Self::interval_from_now(cadence),
        }
    }

    pub(super) fn reconfigure(&mut self, cadence: Duration) {
        if self.cadence != cadence {
            self.cadence = cadence;
            self.interval = Self::interval_from_now(cadence);
        }
    }

    async fn tick(&mut self) {
        self.interval.tick().await;
    }

    fn interval_from_now(cadence: Duration) -> tokio::time::Interval {
        let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + cadence, cadence);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval
    }
}

pub(super) fn animation_cadence(app: &App) -> Duration {
    if app.input.ultrathink.active || app.thinking.active {
        ACTIVE_TICK_CADENCE
    } else {
        IDLE_TICK_CADENCE
    }
}

/// Events selected by [`next_loop_event`] for a single render-loop iteration.
pub(super) enum LoopEvent {
    Terminal(Event),
    TerminalStreamError(io::Error),
    TerminalStreamClosed,
    Tui(TuiEvent),
    TuiChannelClosed,
    Tick,
}

/// Await the next terminal, TUI, or animation event without blocking the
/// render loop on terminal input.
///
/// `terminal_events: None` is the explicitly named headless test seam: it
/// selects only TUI events and ticks. Production and generic callers use
/// [`super::run_inner`], which supplies an [`EventStream`].
pub(super) async fn next_loop_event<S>(
    terminal_events: Option<&mut S>,
    event_rx: &mut TuiEventReceiver,
    scheduler: &mut TickScheduler,
) -> LoopEvent
where
    S: Stream<Item = io::Result<Event>> + Unpin,
{
    match terminal_events {
        Some(terminal_events) => {
            tokio::select! {
                biased;
                _ = scheduler.tick() => LoopEvent::Tick,
                terminal_event = terminal_events.next() => match terminal_event {
                    Some(Ok(event)) => LoopEvent::Terminal(event),
                    Some(Err(error)) => LoopEvent::TerminalStreamError(error),
                    None => LoopEvent::TerminalStreamClosed,
                },
                tui_event = event_rx.recv() => match tui_event {
                    Some(event) => LoopEvent::Tui(event),
                    None => LoopEvent::TuiChannelClosed,
                },
            }
        }
        None => {
            tokio::select! {
                biased;
                _ = scheduler.tick() => LoopEvent::Tick,
                tui_event = event_rx.recv() => match tui_event {
                    Some(event) => LoopEvent::Tui(event),
                    None => LoopEvent::TuiChannelClosed,
                },
            }
        }
    }
}

pub(super) async fn drain_tui_events(
    app: &mut App,
    initial_event: TuiEvent,
    event_rx: &mut TuiEventReceiver,
    input_tx: &crate::app::InputSender,
) -> usize {
    let mut drained = 0;
    let mut next_event = Some(initial_event);

    while drained < MAX_TUI_EVENTS_PER_FRAME {
        let Some(event) = next_event.take().or_else(|| event_rx.try_recv().ok()) else {
            break;
        };
        crate::observability::record_tui_event_drain(event.variant_name());
        super::tui_events::handle_tui_event(app, event, input_tx).await;
        drained += 1;
    }

    drained
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::time::Duration;

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use futures_util::stream;
    use tokio::sync::mpsc;

    use super::*;

    #[tokio::test]
    async fn queued_tui_event_wins_while_terminal_stream_is_pending() {
        let (event_tx, mut event_rx) = crate::event_channel::bounded_tui_event_channel();
        let (input_tx, _input_rx) = mpsc::channel(1);
        let mut terminal_events = stream::pending::<io::Result<Event>>();
        let mut scheduler = TickScheduler::new(Duration::from_secs(60));
        event_tx
            .send(TuiEvent::TextDelta("from-agent".into()))
            .unwrap();

        let event =
            next_loop_event(Some(&mut terminal_events), &mut event_rx, &mut scheduler).await;
        assert!(
            matches!(event, LoopEvent::Tui(TuiEvent::TextDelta(ref text)) if text == "from-agent")
        );

        let mut app = App::new();
        if let LoopEvent::Tui(event) = event {
            super::super::tui_events::handle_tui_event(&mut app, event, &input_tx).await;
        }
        assert!(
            app.output
                .all_lines()
                .iter()
                .any(|line| line.contains("from-agent"))
        );
    }

    #[tokio::test]
    async fn terminal_key_reaches_existing_handler_while_tui_receive_is_pending() {
        let (_event_tx, mut event_rx) = crate::event_channel::bounded_tui_event_channel();
        let (input_tx, mut input_rx) = mpsc::channel(1);
        let mut terminal_events = stream::iter([Ok(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))]);
        let mut scheduler = TickScheduler::new(Duration::from_secs(60));

        let event =
            next_loop_event(Some(&mut terminal_events), &mut event_rx, &mut scheduler).await;
        let mut app = App::new();
        app.input.set_text("typed");
        let keymap = crate::keybindings::KeyMap::default();
        if let LoopEvent::Terminal(event) = event {
            super::super::input::handle_key_event(
                &mut app, event, &input_tx, None, None, None, &keymap,
            )
            .await;
        } else {
            panic!("expected terminal key event");
        }

        assert_eq!(input_rx.try_recv().unwrap(), "typed");
    }

    #[tokio::test]
    async fn terminal_stream_error_is_returned_to_the_live_loop() {
        let (_event_tx, mut event_rx) = crate::event_channel::bounded_tui_event_channel();
        let mut terminal_events = stream::iter([Err(io::Error::other("stream failed"))]);
        let mut scheduler = TickScheduler::new(Duration::from_secs(60));

        let event =
            next_loop_event(Some(&mut terminal_events), &mut event_rx, &mut scheduler).await;
        assert!(
            matches!(event, LoopEvent::TerminalStreamError(error) if error.kind() == io::ErrorKind::Other)
        );
    }

    #[tokio::test]
    async fn closed_terminal_stream_is_returned_to_the_live_loop() {
        let (_event_tx, mut event_rx) = crate::event_channel::bounded_tui_event_channel();
        let mut terminal_events = stream::empty::<io::Result<Event>>();
        let mut scheduler = TickScheduler::new(Duration::from_secs(60));

        let event =
            next_loop_event(Some(&mut terminal_events), &mut event_rx, &mut scheduler).await;
        assert!(matches!(event, LoopEvent::TerminalStreamClosed));
    }

    #[tokio::test]
    async fn live_loop_selector_returns_terminal_input() {
        let (_event_tx, mut event_rx) = crate::event_channel::bounded_tui_event_channel();
        let mut terminal_events = stream::iter([Ok(Event::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
        )))]);
        let mut scheduler = TickScheduler::new(Duration::from_secs(60));

        assert!(matches!(
            next_loop_event(Some(&mut terminal_events), &mut event_rx, &mut scheduler).await,
            LoopEvent::Terminal(Event::Key(KeyEvent {
                code: KeyCode::Char('x'),
                ..
            }))
        ));
    }

    #[tokio::test]
    async fn tui_drain_budget_leaves_backlog_for_next_select() {
        let (event_tx, mut event_rx) = crate::event_channel::bounded_tui_event_channel();
        let (input_tx, _input_rx) = mpsc::channel(1);
        for index in 0..=MAX_TUI_EVENTS_PER_FRAME {
            event_tx
                .send(TuiEvent::TextDelta(index.to_string()))
                .unwrap();
        }
        let initial_event = event_rx.recv().await.unwrap();
        let mut app = App::new();

        let drained = drain_tui_events(&mut app, initial_event, &mut event_rx, &input_tx).await;

        assert_eq!(drained, MAX_TUI_EVENTS_PER_FRAME);
        assert_eq!(event_rx.len(), 1, "one event must remain after the budget");

        let mut terminal_events = stream::iter([Ok(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))]);
        let mut scheduler = TickScheduler::new(Duration::from_secs(60));
        assert!(matches!(
            next_loop_event(Some(&mut terminal_events), &mut event_rx, &mut scheduler).await,
            LoopEvent::Terminal(_)
        ));
        assert_eq!(
            event_rx.len(),
            1,
            "terminal selection must regain control without consuming queued TUI work"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn persistent_tick_scheduler_selects_tick_despite_continuous_tui_events() {
        let (event_tx, mut event_rx) = crate::event_channel::bounded_tui_event_channel();
        let mut terminal_events = stream::pending::<io::Result<Event>>();
        let mut scheduler = TickScheduler::new(Duration::from_millis(250));
        scheduler.reconfigure(Duration::from_millis(80));

        // Each input arrives before the active cadence expires. Rebuilding the
        // scheduler after every selected event would move its deadline to
        // 155ms; the persistent scheduler remains due at 80ms.
        for index in 0..3 {
            event_tx
                .send(TuiEvent::TextDelta(index.to_string()))
                .unwrap();
            assert!(matches!(
                next_loop_event(Some(&mut terminal_events), &mut event_rx, &mut scheduler,).await,
                LoopEvent::Tui(_)
            ));
            tokio::time::advance(Duration::from_millis(25)).await;
        }

        event_tx
            .send(TuiEvent::TextDelta("still-busy".into()))
            .unwrap();
        tokio::time::advance(Duration::from_millis(5)).await;
        assert!(matches!(
            next_loop_event(Some(&mut terminal_events), &mut event_rx, &mut scheduler,).await,
            LoopEvent::Tick
        ));
    }

    #[tokio::test]
    async fn closed_tui_channel_is_returned_to_the_live_loop() {
        let (event_tx, mut event_rx) = crate::event_channel::bounded_tui_event_channel();
        drop(event_tx);
        let mut terminal_events = stream::pending::<io::Result<Event>>();
        let mut scheduler = TickScheduler::new(Duration::from_secs(60));

        let event =
            next_loop_event(Some(&mut terminal_events), &mut event_rx, &mut scheduler).await;
        assert!(matches!(event, LoopEvent::TuiChannelClosed));
    }
}

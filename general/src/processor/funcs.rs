use crate::processor::{ProcessValue, ProcessingState, Processor};
use crate::types::Annotated;

/// Processes the value using the given processor.
#[inline]
pub fn process_value<T, P>(
    annotated: &mut Annotated<T>,
    processor: &mut P,
    state: &ProcessingState<'_>,
) where
    T: ProcessValue,
    P: Processor,
{
    processor.before_process(annotated.0.as_mut(), &mut annotated.1, state);
    annotated.apply(|value, meta| ProcessValue::process_value(value, meta, processor, state))
}

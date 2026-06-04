use super::*;

pub(super) fn merge_session_entities(
    entities: Vec<SessionEntity>,
    sessions: Vec<SessionSummary>,
    field: Rect,
) -> Vec<SessionEntity> {
    let mut existing = existing_entities_by_session_id(entities);
    let mut next = sessions
        .into_iter()
        .map(|session| merge_session_entity(session, &mut existing, field))
        .collect::<Vec<_>>();
    next.sort_by(|a, b| compare_tmux_natural(&a.session, &b.session));
    next
}

fn existing_entities_by_session_id(entities: Vec<SessionEntity>) -> HashMap<String, SessionEntity> {
    entities
        .into_iter()
        .map(|entity| (entity.session.session_id.clone(), entity))
        .collect()
}

fn merge_session_entity(
    session: SessionSummary,
    existing: &mut HashMap<String, SessionEntity>,
    field: Rect,
) -> SessionEntity {
    let Some(mut entity) = existing.remove(&session.session_id) else {
        return SessionEntity::new(session, field);
    };
    entity.session = session;
    entity
}

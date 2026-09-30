//! ADR-0050 in-guest kernel invariant proof used by the historical M4
//! IPC suite. The real ring-3 caller-first regression separately drives
//! SYS_IPC_CALL; this fixture covers ALL four queue states, including
//! Replied/Failed and the typed server reply after caller teardown.
use super::*;

pub(crate) fn stage(caller: u64) -> Result<u32, &'static str> {
    let eid = create_endpoint()?;
    without_interrupts(|| unsafe {
        let ep = &mut (*ENDPOINTS.get())[eid as usize];
        let staged = Cap { obj: CapObj::Image { img_id: 0 }, rights: crate::cap::RIGHTS_READ };
        for (i, state) in [SlotState::Waiting, SlotState::Delivered,
            SlotState::Replied, SlotState::Failed].iter().enumerate() {
            ep.q[i] = CallSlot {
                state: *state, caller, server: sched::current_thread_id(),
                send_cap: staged, reply_cap: staged, landed_cap: CAP_NONE,
                msg: [0x51; MSG_BYTES], reply_msg: [0xA7; MSG_BYTES],
                ..EMPTY_SLOT
            };
        }
    });
    Ok(eid)
}

pub(crate) fn verify(eid: u32) -> Result<(), &'static str> {
    let cleared = without_interrupts(|| unsafe {
        let ep = &(*ENDPOINTS.get())[eid as usize];
        ep.live && !ep.orphaned && ep.server == NO_TID &&
            ep.q.iter().all(|s| s.state == SlotState::Empty && s.caller == 0
                && s.server == NO_TID && s.landed_cap == CAP_NONE
                && matches!(s.send_cap.obj, CapObj::None)
                && matches!(s.reply_cap.obj, CapObj::None)
                && s.msg == [0; MSG_BYTES] && s.reply_msg == [0; MSG_BYTES])
    });
    if !cleared { return Err("caller sweep left queued/staged authority or orphaned server"); }
    if reply(eid, [0, 0], Some(Cap { obj: CapObj::Image { img_id: 1 },
        rights: crate::cap::RIGHTS_READ }), [0xFE; MSG_BYTES]) != Err(STATUS_BAD_ARG) {
        return Err("server reply after caller teardown did not return typed refusal");
    }
    destroy_endpoint(eid)?;
    let recycled = create_endpoint()?;
    if recycled != eid { return Err("abandoned caller queue prevented endpoint reuse"); }
    destroy_endpoint(recycled)?;
    Ok(())
}

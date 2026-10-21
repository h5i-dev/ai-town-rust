import AiTownKernel
import H5iAppLib
/-!
# Exclusivity, membership, collision, and authorization

Each theorem is a postcondition of the extracted kernel, and each is false
for the mutant of the same check in `h5i-app.toml`.
-/
open Aeneas Aeneas.Std Result ai_town_kernel H5iAppLib
open List

namespace ai_town_kernel

/-! ## Index loops -/

@[step] theorem member_index_spec (v : alloc.vec.Vec Member) (player : U64) :
    rules.member_index v player ⦃ o =>
      v.val.findIdx? (fun m => decide (m.player = player)) = o.map (·.val) ⦄ := by
  unfold rules.member_index rules.member_index_loop
  apply WP.spec_mono (loop_search v.val (fun m => decide (m.player = player))
      (fun o : Option Usize => o.map (·.val)) (fun n _ => some n) none
      _ ?_ 0#usize (by simp))
  · intro r hr
    exact (search_findIdx _ _ _ hr).symm
  · intro j hj
    unfold rules.member_index_loop.body
    h5i_step

@[step] theorem player_index_spec (v : alloc.vec.Vec Player) (id : U64) :
    rules.player_index v id ⦃ o =>
      v.val.findIdx? (fun p => decide (p.id = id)) = o.map (·.val) ⦄ := by
  unfold rules.player_index rules.player_index_loop
  apply WP.spec_mono (loop_search v.val (fun p => decide (p.id = id))
      (fun o : Option Usize => o.map (·.val)) (fun n _ => some n) none
      _ ?_ 0#usize (by simp))
  · intro r hr
    exact (search_findIdx _ _ _ hr).symm
  · intro j hj
    unfold rules.player_index_loop.body
    h5i_step

@[step] theorem player_by_owner_spec (v : alloc.vec.Vec Player) (owner : U64) :
    rules.player_by_owner v owner ⦃ o =>
      v.val.findIdx? (fun p => decide (p.owner = owner)) = o.map (·.val) ⦄ := by
  unfold rules.player_by_owner rules.player_by_owner_loop
  apply WP.spec_mono (loop_search v.val (fun p => decide (p.owner = owner))
      (fun o : Option Usize => o.map (·.val)) (fun n _ => some n) none
      _ ?_ 0#usize (by simp))
  · intro r hr
    exact (search_findIdx _ _ _ hr).symm
  · intro j hj
    unfold rules.player_by_owner_loop.body
    h5i_step

/-- The id `own_player` returns belongs to `owner`. `none` is not found. -/
@[step] theorem own_player_spec (s : Snapshot) (owner : U64) :
    rules.own_player s owner ⦃ r =>
      match s.players.val.findIdx? (fun p => decide (p.owner = owner)) with
      | none => r = .Err .NotFound
      | some n => ∃ hn : n < s.players.val.length,
          r = .Ok (s.players.val[n]'hn).id ∧ (s.players.val[n]'hn).owner = owner ⦄ := by
  unfold rules.own_player
  simp only [alloc.vec.Vec.index_slice_index]
  apply WP.spec_bind (player_by_owner_spec s.players owner)
  intro o ho
  cases o with
  | none =>
    simp only [Option.map_none] at ho
    simp only [ho, WP.spec_ok]
  | some i =>
    simp only [Option.map_some] at ho
    have hex := List.findIdx?_eq_some_iff_getElem.mp ho
    obtain ⟨hlt, hp, -⟩ := hex
    have hown : (s.players.val[i.val]'hlt).owner = owner := by
      simpa [decide_eq_true_eq] using hp
    apply WP.spec_bind (alloc.vec.Vec.index_usize_spec s.players i hlt)
    intro p hp'
    rw [ho]
    simp only [WP.spec_ok, hp']
    refine ⟨hlt, ?_, hown⟩
    trivial

/-! ## Authorization -/

/-- The player id resolved for `owner` is a player that owner owns. -/
theorem motion_isolated_for_player (s : Snapshot) (owner : U64) :
    rules.own_player s owner ⦃ r =>
      ∀ id, r = .Ok id →
        ∃ n, ∃ hn : n < s.players.val.length,
          (s.players.val[n]'hn).id = id ∧ (s.players.val[n]'hn).owner = owner ⦄ := by
  apply WP.spec_mono (own_player_spec s owner)
  intro r hr id hid
  cases h : s.players.val.findIdx? (fun p => decide (p.owner = owner)) with
  | none =>
    rw [h] at hr
    simp [hr] at hid
  | some n =>
    rw [h] at hr
    obtain ⟨hn, hr, hown⟩ := hr
    subst hr
    injection hid with hid
    exact ⟨n, hn, hid, hown⟩

/-! ## Exclusivity -/

/-- Starting a conversation is refused when the caller is already a member. -/
theorem exclusive_preserved
    (s : Snapshot) (owner invitee now id : U64) (ii : Nat)
    (hii : ii < s.players.val.length)
    (hOwn : s.players.val.findIdx? (fun p => decide (p.owner = owner)) = some ii)
    (hId : (s.players.val[ii]).id = id)
    (hNe : id ≠ invitee)
    (hGuest : (s.players.val.findIdx? (fun p => decide (p.id = invitee))).isSome = true)
    (hMem : (s.members.val.findIdx? (fun m => decide (m.player = id))).isSome = true) :
    rules.dispatch_start s owner invitee now ⦃ r => r.1 = .Err .Busy ⦄ := by
  unfold rules.dispatch_start
  h5i_steps
  all_goals try simp_all
  case h2 =>
    obtain ⟨m, hm, heq⟩ := hMem
    exfalso
    exact o1_post m hm (UScalar.val_eq_of_eq heq)

/-! ## Membership -/

/-- A first conversation started by `owner` with `invitee` has exactly two
members: the caller walking over, and the invitee invited. The creator is
the caller. -/
theorem members_consistent_preserved
    (s : Snapshot) (owner invitee now id : U64) (ii gi : Nat)
    (hEmptyM : s.members.val = [])
    (hEmptyC : s.conversations.val = [])
    (hii : ii < s.players.val.length)
    (hOwn : s.players.val.findIdx? (fun p => decide (p.owner = owner)) = some ii)
    (hId : (s.players.val[ii]).id = id)
    (hGuest : s.players.val.findIdx? (fun p => decide (p.id = invitee)) = some gi)
    (hNe : id ≠ invitee)
    (hNext : s.meta.next_id.val + 1 < 2 ^ 64) :
    rules.dispatch_start s owner invitee now ⦃ r =>
      ∃ cid m0 m1 c s', r = (.Ok (.Conversation cid), s') ∧
        s'.members.val = [m0, m1] ∧
        s'.conversations.val = [c] ∧
        m0.player = id ∧ m0.status = ST_WALKING ∧
        m1.player = invitee ∧ m1.status = ST_INVITED ∧
        m0.conversation = cid ∧ m1.conversation = cid ∧
        c.creator = id ∧ c.id = cid ⦄ := by
  unfold rules.dispatch_start
  h5i_steps
  all_goals try simp_all
  case h2 =>
    unfold rules.alloc
    h5i_steps
    all_goals try (simp only [core.num.U64.MAX, U64.rMax] at *; scalar_tac)
    all_goals try
      (rw [‹o3 = some i›] at o3_post
       simp only [Option.map_some] at o3_post
       exact (List.findIdx?_eq_some_iff_getElem.mp o3_post).1)
    all_goals
      let m0 : Member :=
        { conversation := s.meta.next_id, player := id, invited := now, status := ST_WALKING, started := 0#u64 }
      let m1 : Member :=
        { conversation := s.meta.next_id, player := invitee, invited := now, status := ST_INVITED, started := 0#u64 }
      let c : Conversation :=
        { id := s.meta.next_id, creator := id, created := now, typing_player := 0#u64, typing_since := 0#u64,
          last_author := 0#u64, last_at := 0#u64, num_messages := 0#u64 }
      have hm : v4.val = [m0, m1] := by simp [v4_post, v3_post, hEmptyM, m0, m1]
      have hc : v2.val = [c] := by simp [v2_post, hEmptyC, c]
      let base :=
        { s with «meta» := { s.meta with next_id := x }, conversations := v2, members := v4 }
      first
        | exact ⟨m0, m1, c, base, rfl, rfl, hm, hc, rfl, rfl, rfl, rfl, rfl, rfl, rfl⟩
        | exact ⟨m0, m1, c,
            { base with players := index_mut_back { p with last_input := now } },
            rfl, rfl, hm, hc, rfl, rfl, rfl, rfl, rfl, rfl, rfl⟩

/-! ## Collision -/

def absNat (a b : Nat) : Nat := if b ≤ a then a - b else b - a

/-- Squared milli-tile distance, saturating the way `geom.dist_sq` does. -/
def distNat (ax ay bx cy : U32) : Nat :=
  let dx := absNat ax.val bx.val
  let dy := absNat ay.val cy.val
  if 2000000 < dx then 2 ^ 64 - 1
  else if 2000000 < dy then 2 ^ 64 - 1
  else dx * dx + dy * dy

@[step] theorem dist_sq_spec (ax ay bx cy : U32) :
    geom.dist_sq ax ay bx cy ⦃ r => r.val = distNat ax ay bx cy ⦄ := by
  unfold geom.dist_sq geom.abs_diff distNat absNat
  h5i_steps
  all_goals try (simp only [core.num.U64.MAX, U64.rMax] at *; scalar_tac)

@[step] theorem collision_sq_spec : COLLISION_SQ ⦃ r => r.val = 562500 ⦄ := by
  unfold COLLISION_SQ
  step*

/-- Another player strictly inside the collision radius. -/
def tooClose (ignore : U64) (x y : U32) (p : Player) : Bool :=
  decide (p.id ≠ ignore ∧ distNat p.x p.y x y < 562500)

@[step] theorem occupied_loop_spec
    (v : alloc.vec.Vec Player) (x y : U32) (ignore : U64) :
    rules.occupied_loop v x y ignore 0#usize ⦃ b =>
      b = v.val.any (tooClose ignore x y) ⦄ := by
  unfold rules.occupied_loop
  apply WP.spec_mono (loop_search v.val (tooClose ignore x y) id
      (fun _ _ => true) false (rules.occupied_loop.body v x y ignore) ?_
      0#usize (by simp))
  · intro r hr
    simp only [id] at hr
    exact search_any _ _ _ hr
  · intro j hj
    unfold rules.occupied_loop.body tooClose
    h5i_step

/-- A walkable point that another player stands inside the collision radius
of is occupied. Movement refuses to step onto an occupied point. -/
theorem collision_safe_preserved
    (s : Snapshot) (x y : U32) (ignore : U64)
    (hMask : geom.mask_blocked (alloc.vec.Vec.deref s.terrain.mask)
        s.meta.width s.meta.height x y ⦃ b => b = false ⦄)
    (hClose : s.players.val.any (tooClose ignore x y) = true) :
    rules.occupied s x y ignore ⦃ b => b = true ⦄ := by
  unfold rules.occupied
  apply WP.spec_bind hMask
  intro b hb
  subst hb
  simp only [Bool.false_eq_true, ite_false]
  apply WP.spec_mono (occupied_loop_spec s.players x y ignore)
  intro r hr
  rw [hr, hClose]

end ai_town_kernel

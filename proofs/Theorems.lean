import AiTownKernel
import H5iAppLib
/-!
# Exclusivity, membership, collision, and authorization

Each theorem is a postcondition of the extracted kernel, and each is false
for the mutant of the same check in `h5i-app.toml`.

The default tick is two-phase. Every agent proposes from the previous
positions, then keeps that candidate only when no better-priority proposal
(lower player id, then lower index) lies inside the collision radius.
`two_phase_apart` is that rule: a collision-free round stays collision-free,
and an accepted landing was not refused as blocked.
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

/-! ## Per-agent step -/

/-- A blocked candidate is refused. The agent keeps its position. -/
theorem step_never_blocked
    (x y face_dx face_dy : U32) (moving : Bool)
    (cand_x cand_y cand_fx cand_fy cand_speed : U32)
    (mask : Slice U8) (width height : U32)
    (head next xs ys : Slice U32) (skip : U32)
    (hMask : geom.mask_blocked mask width height cand_x cand_y ⦃ b => b = true ⦄) :
    step.agent_step x y face_dx face_dy moving cand_x cand_y cand_fx cand_fy cand_speed
      mask width height head next xs ys skip ⦃ land =>
        land.moved = false ∧ land.x = x ∧ land.y = y ∧ land.speed = 0#u32 ⦄ := by
  unfold step.agent_step
  apply WP.spec_bind hMask
  intro tile htile
  subst htile
  h5i_steps

/-- An open tile that the spatial grid reports as occupied is refused. -/
theorem step_never_overlap
    (x y face_dx face_dy : U32) (moving : Bool)
    (cand_x cand_y cand_fx cand_fy cand_speed : U32)
    (mask : Slice U8) (width height : U32)
    (head next xs ys : Slice U32) (skip : U32)
    (hMask : geom.mask_blocked mask width height cand_x cand_y ⦃ b => b = false ⦄)
    (hHit : geom.grid_hit width height head next xs ys cand_x cand_y skip ⦃ b => b = true ⦄) :
    step.agent_step x y face_dx face_dy moving cand_x cand_y cand_fx cand_fy cand_speed
      mask width height head next xs ys skip ⦃ land =>
        land.moved = false ∧ land.x = x ∧ land.y = y ⦄ := by
  unfold step.agent_step
  apply WP.spec_bind hMask
  intro tile htile
  subst htile
  simp only [Bool.false_eq_true, ite_false]
  apply WP.spec_bind hHit
  intro hit hhit
  subst hhit
  h5i_steps

/-- The only accepted landing is the candidate, and only when the agent is
moving, the tile is open, and the grid reports nobody else inside the radius. -/
theorem step_clear_moves
    (x y face_dx face_dy : U32)
    (cand_x cand_y cand_fx cand_fy cand_speed : U32)
    (mask : Slice U8) (width height : U32)
    (head next xs ys : Slice U32) (skip : U32)
    (hMask : geom.mask_blocked mask width height cand_x cand_y ⦃ b => b = false ⦄)
    (hHit : geom.grid_hit width height head next xs ys cand_x cand_y skip ⦃ b => b = false ⦄) :
    step.agent_step x y face_dx face_dy true cand_x cand_y cand_fx cand_fy cand_speed
      mask width height head next xs ys skip ⦃ land =>
        land.moved = true ∧ land.x = cand_x ∧ land.y = cand_y ∧
          land.face_dx = cand_fx ∧ land.face_dy = cand_fy ∧ land.speed = cand_speed ⦄ := by
  unfold step.agent_step
  apply WP.spec_bind hMask
  intro tile htile
  subst htile
  simp only [Bool.false_eq_true, ite_false]
  apply WP.spec_bind hHit
  intro hit hhit
  subst hhit
  h5i_steps

/-! ## Batched tick

`grid_hit` adds coordinates with a fallible `+`, so a total spec for every
map would be false. The batch theorems assume `tick_moves` returns and keeps
the slice lengths. Each landing is still the decision proved above: a blocked
or crowded candidate is refused, and an accepted landing is that candidate.
-/

open Aeneas.Std.alloc.vec

/-- Ids and owners, index by index. Length is a separate fact. -/
def idsOwners (v v0 : alloc.vec.Vec Player) : Prop :=
  ∀ j (hv : j < v.val.length) (hv0 : j < v0.val.length),
    (v.val[j]'hv).id = (v0.val[j]'hv0).id ∧
      (v.val[j]'hv).owner = (v0.val[j]'hv0).owner

/-- Overwriting one player keeps every id and every owner. -/
theorem roster_set
    (v0 v : alloc.vec.Vec Player) (i : Usize) (p : Player)
    (hlen : v.val.length = v0.val.length)
    (hi : i.val < v.val.length)
    (hp : p.id = (v.val[i.val]'hi).id ∧ p.owner = (v.val[i.val]'hi).owner)
    (hids : idsOwners v v0) :
    (v.set i p).val.length = v0.val.length ∧ idsOwners (v.set i p) v0 := by
  have hval : (↑(v.set i p) : List Player) = v.val.set i.val p := Vec.set_val_eq v i p
  constructor
  · rw [hval, List.length_set]
    exact hlen
  · intro j hj hj0
    have hjv : j < v.val.length := by
      have hjSet : j < (v.val.set i.val p).length := by simpa [hval] using hj
      simpa [List.length_set] using hjSet
    have hleft := getElem_congr_coll (c := (↑(v.set i p) : List Player)) hval (i := j) (w := hj)
    have hget := List.getElem_set (l := v.val) (i := i.val) (j := j) (a := p)
      (h := by simpa [List.length_set] using hjv)
    rw [hleft, hget]
    by_cases hij : i.val = j
    · simp only [if_pos hij]
      have hold := hids i.val hi (hij ▸ hj0)
      have hidx := getElem_congr_idx (c := v0.val) hij (w := hij ▸ hj0)
      rw [hp.1, hp.2, hold.1, hold.2, hidx]
      exact ⟨rfl, rfl⟩
    · simp only [if_neg hij]
      exact hids j hjv hj0

/-- Copying positions out of the player vector keeps one coordinate per player. -/
theorem tick_players_loop0_length
    (v : alloc.vec.Vec Player) (n : Usize) (xs ys : alloc.vec.Vec U32) (i : Usize)
    (hn : n.val = v.val.length) (hi : i.val ≤ n.val)
    (hx : xs.val.length = i.val) (hy : ys.val.length = i.val) :
    step.tick_players_loop0 v n xs ys i ⦃ r =>
      r.1.val.length = n.val ∧ r.2.val.length = n.val ⦄ := by
  unfold step.tick_players_loop0
  apply loop.spec_decr_nat
    (measure := fun (x : alloc.vec.Vec U32 × alloc.vec.Vec U32 × Usize) => n.val - x.2.2.val)
    (inv := fun x => x.2.2.val ≤ n.val ∧ x.1.val.length = x.2.2.val ∧ x.2.1.val.length = x.2.2.val)
    (post := fun r => r.1.val.length = n.val ∧ r.2.val.length = n.val)
    (body := fun (xs, ys, i) => step.tick_players_loop0.body v n xs ys i)
  · rintro ⟨xs, ys, i⟩ ⟨hi, hx, hy⟩
    simp only at hi hx hy
    unfold step.tick_players_loop0.body
    simp only [Vec.index_slice_index]
    split
    · next hlt =>
      have hltN : i.val < n.val := by
        rw [UScalar.lt_equiv] at hlt
        exact hlt
      have hv : i.val < v.val.length := by
        rw [hn] at hltN
        exact hltN
      apply WP.spec_bind (Vec.index_usize_spec v i hv)
      intro p _hp
      have hone : (1#usize).val = 1 := by scalar_tac
      have hvmax := Vec.len_ineq v
      have hxpush : xs.val.length < Usize.max := by
        rw [hx]
        omega
      apply WP.spec_bind (Vec.push_spec xs p.x hxpush)
      intro xs1 hxs1
      have hypush : ys.val.length < Usize.max := by
        rw [hy]
        omega
      apply WP.spec_bind (Vec.push_spec ys p.y hypush)
      intro ys1 hys1
      have hadd : i.val + (1#usize).val ≤ Usize.max := by
        rw [hone]
        omega
      apply WP.spec_bind (Usize.add_spec hadd)
      intro i1 hi1
      have hval : i1.val = i.val + 1 := by
        simpa [hone] using hi1
      simp only [WP.spec_ok]
      refine ⟨⟨?_, ?_, ?_⟩, ?_⟩
      · omega
      · rw [hxs1, List.length_append, hx, hval]
        simp
      · rw [hys1, List.length_append, hy, hval]
        simp
      · omega
    · next hge =>
      simp only [WP.spec_ok]
      rw [UScalar.lt_equiv] at hge
      have heqi : i.val = n.val := by omega
      rw [hx, hy, heqi]
      exact ⟨rfl, rfl⟩
  · exact ⟨hi, hx, hy⟩

/-- Writing the landed coordinates back changes `x` and `y` only. -/
theorem tick_players_loop1_roster
    (n : Usize) (xs ys : alloc.vec.Vec U32) (v0 v : alloc.vec.Vec Player) (i : Usize)
    (hn : n.val = v0.val.length)
    (hxs : n.val ≤ xs.val.length) (hys : n.val ≤ ys.val.length)
    (hlen : v.val.length = v0.val.length) (hi : i.val ≤ n.val)
    (hids : idsOwners v v0) :
    step.tick_players_loop1 v n xs ys i ⦃ v' =>
      v'.val.length = v0.val.length ∧ idsOwners v' v0 ⦄ := by
  unfold step.tick_players_loop1
  apply loop.spec_decr_nat
    (measure := fun (x : alloc.vec.Vec Player × Usize) => n.val - x.2.val)
    (inv := fun x => x.2.val ≤ n.val ∧ x.1.val.length = v0.val.length ∧ idsOwners x.1 v0)
    (post := fun v' => v'.val.length = v0.val.length ∧ idsOwners v' v0)
    (body := fun (v, i) => step.tick_players_loop1.body n xs ys v i)
  · rintro ⟨v, i⟩ ⟨hi, hlen, hids⟩
    simp only at hi hlen hids
    have _ := hxs
    have _ := hys
    unfold step.tick_players_loop1.body
    simp only [Vec.index_slice_index, Vec.index_mut_slice_index]
    split
    · next hlt =>
      have hltN : i.val < n.val := by
        rw [UScalar.lt_equiv] at hlt
        exact hlt
      have hvi : i.val < v.val.length := by
        rw [hlen, ← hn]
        exact hltN
      split
      · next hxsLt =>
        have hxi : i.val < xs.val.length := by
          rw [UScalar.lt_equiv] at hxsLt
          simpa [Vec.len_val, Vec.length] using hxsLt
        split
        · next hysLt =>
          have hyi : i.val < ys.val.length := by
            rw [UScalar.lt_equiv] at hysLt
            simpa [Vec.len_val, Vec.length] using hysLt
          apply WP.spec_bind (Vec.index_usize_spec xs i hxi)
          intro xv _hxv
          apply WP.spec_bind (Vec.index_mut_usize_spec v i hvi)
          intro got hgot
          simp only [WP.uncurry'_eq] at hgot
          rcases got with ⟨p, back⟩
          dsimp at hgot
          apply WP.spec_bind (Vec.index_usize_spec ys i hyi)
          intro yv _hyv
          have hp : p = v.val[i.val] := hgot.1
          have hback : back = v.set i := hgot.2
          rw [hback]
          have hxPlayer : ({ p with x := xv }).id = (v.val[i.val]'hvi).id ∧
              ({ p with x := xv }).owner = (v.val[i.val]'hvi).owner := by
            rw [hp]
            exact ⟨rfl, rfl⟩
          have hlen1 := roster_set v0 v i { p with x := xv } hlen hvi hxPlayer hids
          have hiSet : i.val < ((v.set i) { p with x := xv }).val.length := by
            rw [hlen1.1, ← hlen]
            exact hvi
          apply WP.spec_bind
            (Vec.index_mut_usize_spec ((v.set i) { p with x := xv }) i hiSet)
          intro got1 hgot1
          simp only [WP.uncurry'_eq] at hgot1
          rcases got1 with ⟨p1, back1⟩
          dsimp at hgot1
          have hone : (1#usize).val = 1 := by scalar_tac
          have hadd : i.val + (1#usize).val ≤ Usize.max := by
            have hvmax := Vec.len_ineq v0
            rw [hone]
            omega
          apply WP.spec_bind (Usize.add_spec hadd)
          intro i1 hi1
          have hstep : i1.val = i.val + 1 := by
            simpa [hone] using hi1
          have hp1 : p1 = ((v.set i) { p with x := xv }).val[i.val] := hgot1.1
          have hback1 : back1 = ((v.set i) { p with x := xv }).set i := hgot1.2
          have hyPlayer : ({ p1 with y := yv }).id =
                (((v.set i) { p with x := xv }).val[i.val]'hiSet).id ∧
              ({ p1 with y := yv }).owner =
                (((v.set i) { p with x := xv }).val[i.val]'hiSet).owner := by
            rw [hp1]
            exact ⟨rfl, rfl⟩
          have hlen2 := roster_set v0 ((v.set i) { p with x := xv }) i
            { p1 with y := yv } hlen1.1 hiSet hyPlayer hlen1.2
          simp only [WP.spec_ok, hback1]
          refine ⟨⟨?_, hlen2.1, hlen2.2⟩, ?_⟩
          · omega
          · omega
        · simp only [WP.spec_ok]
          exact ⟨hlen, hids⟩
      · simp only [WP.spec_ok]
        exact ⟨hlen, hids⟩
    · simp only [WP.spec_ok]
      exact ⟨hlen, hids⟩
  · exact ⟨hi, hlen, hids⟩

/-- `tick_moves` is assumed to succeed and to keep both coordinate slices at
the player count. Its body calls `grid_hit`. -/
abbrev MovesOk (s : Snapshot) (moving cand_x cand_y head next : Slice U32) : Prop :=
  ∀ (xs ys : Slice U32),
    xs.val.length = s.players.val.length →
    ys.val.length = s.players.val.length →
    step.tick_moves xs ys moving cand_x cand_y
      (alloc.vec.Vec.deref s.terrain.mask) s.meta.width s.meta.height head next
      ⦃ r => r.1 = true ∧ r.2.1.val.length = s.players.val.length ∧
        r.2.2.val.length = s.players.val.length ⦄

/-- One movement pass keeps every field except player coordinates, and it
keeps ids and owners. -/
theorem tick_players_keeps_roster
    (s : Snapshot) (moving cand_x cand_y head next : Slice U32)
    (hStep : MovesOk s moving cand_x cand_y head next) :
    step.tick_players s moving cand_x cand_y head next ⦃ s' =>
      s'.meta = s.meta ∧ s'.terrain = s.terrain ∧ s'.paths = s.paths ∧
      s'.conversations = s.conversations ∧ s'.members = s.members ∧
      s'.agents = s.agents ∧ s'.memories = s.memories ∧ s'.messages = s.messages ∧
      s'.players.val.length = s.players.val.length ∧ idsOwners s'.players s.players ⦄ := by
  unfold step.tick_players
  set n := alloc.vec.Vec.len s.players with hn
  have hnval : n.val = s.players.val.length := by simp [hn]
  have hempty : (alloc.vec.Vec.new U32).val.length = 0 := by simp [alloc.vec.Vec.new, Vec.from_val]
  have hzero : (0#usize).val = 0 := by scalar_tac
  apply WP.spec_bind (tick_players_loop0_length s.players n
      (alloc.vec.Vec.new U32) (alloc.vec.Vec.new U32) 0#usize
      hnval (by rw [hzero]; exact Nat.zero_le _)
      (by rw [hempty, hzero]) (by rw [hempty, hzero]))
  intro copied hcopied
  obtain ⟨xs, ys⟩ := copied
  obtain ⟨hxs, hys⟩ := hcopied
  simp only [lift, alloc.vec.Vec.deref_mut, bind_ok]
  have hxsL : xs.slice.val.length = s.players.val.length := by
    simpa [Vec.val, hnval] using hxs
  have hysL : ys.slice.val.length = s.players.val.length := by
    simpa [Vec.val, hnval] using hys
  apply WP.spec_bind (hStep xs.slice ys.slice hxsL hysL)
  intro moved hmoved
  obtain ⟨_, xs', ys'⟩ := moved
  obtain ⟨_, hxs', hys'⟩ := hmoved
  have hxlen : ({ slice := xs' } : alloc.vec.Vec U32).val.length = s.players.val.length := by
    simpa [Vec.val] using hxs'
  have hylen : ({ slice := ys' } : alloc.vec.Vec U32).val.length = s.players.val.length := by
    simpa [Vec.val] using hys'
  apply WP.spec_bind (tick_players_loop1_roster n { slice := xs' } { slice := ys' }
      s.players s.players 0#usize hnval
      (Nat.le_of_eq (by rw [hxlen, hnval]))
      (Nat.le_of_eq (by rw [hylen, hnval]))
      rfl (by rw [hzero]; exact Nat.zero_le _)
      (by intro j hj hj0; exact ⟨rfl, rfl⟩))
  intro v hv
  simp only [WP.spec_ok]
  obtain ⟨hlen, hids⟩ := hv
  refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, hlen, hids⟩
  all_goals first | rfl | trivial

/-- Exclusivity reads the member list only. -/
theorem exclusive_of_members
    (s t : Snapshot) (h : s.members = t.members) :
    rules.exclusive s = rules.exclusive t := by
  unfold rules.exclusive rules.exclusive_loop0 rules.exclusive_loop0.body
  rw [h]

/-- Two player lists with the same predicate at each index give the same `findIdx?`. -/
theorem findIdx?_ids
    (v w : List Player) (P : Player → Bool)
    (hlen : v.length = w.length)
    (h : ∀ j (hj : j < v.length), P (v[j]'hj) = P (w[j]'(by omega))) :
    v.findIdx? P = w.findIdx? P := by
  induction v generalizing w with
  | nil =>
    cases w with
    | nil => rfl
    | cons _ _ => simp at hlen
  | cons a vs ih =>
    cases w with
    | nil => simp at hlen
    | cons b ws =>
      have hlen' : vs.length = ws.length := by simpa using hlen
      have hab : P a = P b := by
        have h0 := h 0 (by simp)
        rw [List.getElem_cons_zero, List.getElem_cons_zero] at h0
        exact h0
      simp only [List.findIdx?_cons, hab]
      split
      · rfl
      · apply congrArg (Option.map (fun n => n + 1))
        refine ih ws hlen' ?_
        intro j hj
        have h1 := h (j + 1) (by simp; omega)
        rw [List.getElem_cons_succ, List.getElem_cons_succ] at h1
        exact h1

theorem option_usize_ext {a b : Option Usize}
    (h : a.map (·.val) = b.map (·.val)) : a = b := by
  cases a with
  | none =>
    cases b with
    | none => rfl
    | some b =>
      simp only [Option.map_none, Option.map_some] at h
      cases h
  | some a =>
    cases b with
    | none =>
      simp only [Option.map_none, Option.map_some] at h
      cases h
    | some b =>
      simp only [Option.map_some] at h
      injection h with hval
      exact congrArg some (UScalar.val_eq_imp a b hval)

/-- `player_index` is determined by the ids. -/
theorem player_index_of_ids
    (v w : alloc.vec.Vec Player) (id : U64)
    (hlen : v.val.length = w.val.length)
    (hids : ∀ j (hj : j < v.val.length), (v.val[j]).id = (w.val[j]'(by omega)).id) :
    rules.player_index v id = rules.player_index w id := by
  obtain ⟨ov, hv, hov⟩ := (WP.spec_equiv_exists _ _).1 (player_index_spec v id)
  obtain ⟨ow, hw, how⟩ := (WP.spec_equiv_exists _ _).1 (player_index_spec w id)
  have hfind := findIdx?_ids v.val w.val (fun p => decide (p.id = id)) hlen
    (by intro j hj; rw [hids j hj])
  rw [hv, hw]
  apply congrArg ok
  apply option_usize_ext
  rw [← hov, ← how]
  exact hfind

/-- The player id `own_player` returns is determined by ids and owners. -/
theorem own_player_of_ids
    (s t : Snapshot) (owner : U64)
    (hlen : s.players.val.length = t.players.val.length)
    (hids : ∀ j (hj : j < s.players.val.length),
      (s.players.val[j]).id = (t.players.val[j]'(by omega)).id ∧
      (s.players.val[j]).owner = (t.players.val[j]'(by omega)).owner) :
    rules.own_player s owner = rules.own_player t owner := by
  obtain ⟨rs, hs, hsP⟩ := (WP.spec_equiv_exists _ _).1 (own_player_spec s owner)
  obtain ⟨rt, ht, htP⟩ := (WP.spec_equiv_exists _ _).1 (own_player_spec t owner)
  have hfind := findIdx?_ids s.players.val t.players.val
      (fun p => decide (p.owner = owner)) hlen
      (by intro j hj; rw [(hids j hj).2])
  rw [hs, ht]
  rw [hfind] at hsP
  cases hidx : t.players.val.findIdx? (fun p => decide (p.owner = owner)) with
  | none =>
    simp [hidx] at hsP htP
    rw [hsP, htP]
  | some n =>
    simp [hidx] at hsP htP
    obtain ⟨hn, hsP, _⟩ := hsP
    obtain ⟨hn', htP, _⟩ := htP
    rw [hsP, htP, (hids n hn).1]

/-- Membership consistency reads conversations, members, and player ids. -/
theorem members_consistent_of_roster
    (s t : Snapshot)
    (hC : s.conversations = t.conversations)
    (hM : s.members = t.members)
    (hP : rules.player_index s.players = rules.player_index t.players) :
    rules.members_consistent s = rules.members_consistent t := by
  unfold rules.members_consistent rules.members_consistent_loop0 rules.members_consistent_loop0.body
  rw [hC, hM, hP]

/-- A movement pass that keeps members keeps the exclusivity verdict. -/
theorem tick_players_preserves_exclusive
    (s : Snapshot) (moving cand_x cand_y head next : Slice U32)
    (hStep : MovesOk s moving cand_x cand_y head next)
    (hEx : rules.exclusive s ⦃ b => b = true ⦄) :
    step.tick_players s moving cand_x cand_y head next ⦃ s' =>
      rules.exclusive s' ⦃ b => b = true ⦄ ⦄ := by
  apply WP.spec_mono (tick_players_keeps_roster s moving cand_x cand_y head next hStep)
  intro s' hs
  rw [exclusive_of_members s' s hs.2.2.2.2.1]
  exact hEx

/-- A movement pass that keeps ids, members, and conversations keeps the
membership verdict. -/
theorem tick_players_preserves_members
    (s : Snapshot) (moving cand_x cand_y head next : Slice U32)
    (hStep : MovesOk s moving cand_x cand_y head next)
    (hMem : rules.members_consistent s ⦃ b => b = true ⦄) :
    step.tick_players s moving cand_x cand_y head next ⦃ s' =>
      rules.members_consistent s' ⦃ b => b = true ⦄ ⦄ := by
  apply WP.spec_mono (tick_players_keeps_roster s moving cand_x cand_y head next hStep)
  intro s' hs
  have hlen : s'.players.val.length = s.players.val.length := hs.2.2.2.2.2.2.2.2.1
  have hids : idsOwners s'.players s.players := hs.2.2.2.2.2.2.2.2.2
  have hP : rules.player_index s'.players = rules.player_index s.players := by
    funext id
    exact player_index_of_ids s'.players s.players id hlen
      (by intro j hj; exact (hids j hj (by omega)).1)
  rw [members_consistent_of_roster s' s hs.2.2.2.1 hs.2.2.2.2.1 hP]
  exact hMem

/-- A movement pass that keeps ids and owners resolves `own_player` the same
way, so the authorization fact `motion_isolated_for_player` still holds and
names the same player. -/
theorem tick_players_preserves_own_player
    (s : Snapshot) (moving cand_x cand_y head next : Slice U32)
    (hStep : MovesOk s moving cand_x cand_y head next) (owner : U64) :
    step.tick_players s moving cand_x cand_y head next ⦃ s' =>
      rules.own_player s' owner = rules.own_player s owner ∧
      rules.own_player s' owner ⦃ r => ∀ id, r = .Ok id →
        ∃ n, ∃ hn : n < s'.players.val.length,
          (s'.players.val[n]'hn).id = id ∧ (s'.players.val[n]'hn).owner = owner ⦄ ⦄ := by
  apply WP.spec_mono (tick_players_keeps_roster s moving cand_x cand_y head next hStep)
  intro s' hs
  have hlen : s'.players.val.length = s.players.val.length := hs.2.2.2.2.2.2.2.2.1
  have hids : idsOwners s'.players s.players := hs.2.2.2.2.2.2.2.2.2
  refine ⟨?_, ?_⟩
  · exact own_player_of_ids s' s owner hlen
      (by intro j hj; exact hids j hj (by omega))
  · exact motion_isolated_for_player s' owner

/-! ## Two-phase round -/

/-- A blocked candidate is not proposed. The agent keeps its position. -/
theorem propose_never_blocked
    (x y face_dx face_dy : U32) (moving : Bool)
    (cand_x cand_y cand_fx cand_fy cand_speed : U32)
    (mask : Slice U8) (width height : U32)
    (head next xs ys : Slice U32) (skip : U32)
    (hMask : geom.mask_blocked mask width height cand_x cand_y ⦃ b => b = true ⦄) :
    step.propose_one x y face_dx face_dy moving cand_x cand_y cand_fx cand_fy cand_speed
      mask width height head next xs ys skip ⦃ p =>
        p.proposed = false ∧ p.x = x ∧ p.y = y ∧ p.speed = 0#u32 ⦄ := by
  unfold step.propose_one
  apply WP.spec_bind hMask
  intro tile htile
  subst htile
  h5i_steps

/-- An open tile that the spatial grid reports as occupied is not proposed. -/
theorem propose_stays_on_hit
    (x y face_dx face_dy : U32) (moving : Bool)
    (cand_x cand_y cand_fx cand_fy cand_speed : U32)
    (mask : Slice U8) (width height : U32)
    (head next xs ys : Slice U32) (skip : U32)
    (hMask : geom.mask_blocked mask width height cand_x cand_y ⦃ b => b = false ⦄)
    (hHit : geom.grid_hit width height head next xs ys cand_x cand_y skip ⦃ b => b = true ⦄) :
    step.propose_one x y face_dx face_dy moving cand_x cand_y cand_fx cand_fy cand_speed
      mask width height head next xs ys skip ⦃ p =>
        p.proposed = false ∧ p.x = x ∧ p.y = y ⦄ := by
  unfold step.propose_one
  apply WP.spec_bind hMask
  intro tile htile
  subst htile
  simp only [Bool.false_eq_true, ite_false]
  apply WP.spec_bind hHit
  intro hit hhit
  subst hhit
  h5i_steps

/-- The only proposal is the candidate, and only when the agent is moving,
the tile is open, and the grid reports nobody else inside the radius. -/
theorem propose_clear
    (x y face_dx face_dy : U32)
    (cand_x cand_y cand_fx cand_fy cand_speed : U32)
    (mask : Slice U8) (width height : U32)
    (head next xs ys : Slice U32) (skip : U32)
    (hMask : geom.mask_blocked mask width height cand_x cand_y ⦃ b => b = false ⦄)
    (hHit : geom.grid_hit width height head next xs ys cand_x cand_y skip ⦃ b => b = false ⦄) :
    step.propose_one x y face_dx face_dy true cand_x cand_y cand_fx cand_fy cand_speed
      mask width height head next xs ys skip ⦃ p =>
        p.proposed = true ∧ p.x = cand_x ∧ p.y = cand_y ∧
          p.face_dx = cand_fx ∧ p.face_dy = cand_fy ∧ p.speed = cand_speed ⦄ := by
  unfold step.propose_one
  apply WP.spec_bind hMask
  intro tile htile
  subst htile
  simp only [Bool.false_eq_true, ite_false]
  apply WP.spec_bind hHit
  intro hit hhit
  subst hhit
  h5i_steps

/-- An agent that did not propose stays where it was. -/
theorem resolve_stays
    (x y face_dx face_dy cand_x cand_y cand_fx cand_fy cand_speed : U32)
    (self_id : U64) (head next prop_x prop_y : Slice U32) (ids : Slice U64)
    (skip width height : U32) :
    step.resolve_one x y face_dx face_dy cand_x cand_y cand_fx cand_fy cand_speed
      false self_id head next prop_x prop_y ids skip width height ⦃ land =>
        land.moved = false ∧ land.x = x ∧ land.y = y ∧ land.speed = 0#u32 ⦄ := by
  unfold step.resolve_one
  simp only [Bool.false_eq_true, ite_false, WP.spec_ok]
  exact ⟨trivial, trivial, trivial, trivial⟩

/-- A proposal that loses priority inside the radius is refused. -/
theorem resolve_yields
    (x y face_dx face_dy cand_x cand_y cand_fx cand_fy cand_speed : U32)
    (self_id : U64) (head next prop_x prop_y : Slice U32) (ids : Slice U64)
    (skip width height : U32)
    (hYield : geom.grid_yield width height head next prop_x prop_y ids cand_x cand_y self_id skip
      ⦃ b => b = true ⦄) :
    step.resolve_one x y face_dx face_dy cand_x cand_y cand_fx cand_fy cand_speed
      true self_id head next prop_x prop_y ids skip width height ⦃ land =>
        land.moved = false ∧ land.x = x ∧ land.y = y ⦄ := by
  unfold step.resolve_one
  simp only [ite_true]
  apply WP.spec_bind hYield
  intro yielded hy
  subst hy
  h5i_steps

/-- The only accepted landing is the candidate, and only when no better
proposal in the grid is inside the radius. -/
theorem resolve_accepts
    (x y face_dx face_dy cand_x cand_y cand_fx cand_fy cand_speed : U32)
    (self_id : U64) (head next prop_x prop_y : Slice U32) (ids : Slice U64)
    (skip width height : U32)
    (hYield : geom.grid_yield width height head next prop_x prop_y ids cand_x cand_y self_id skip
      ⦃ b => b = false ⦄) :
    step.resolve_one x y face_dx face_dy cand_x cand_y cand_fx cand_fy cand_speed
      true self_id head next prop_x prop_y ids skip width height ⦃ land =>
        land.moved = true ∧ land.x = cand_x ∧ land.y = cand_y ∧
          land.face_dx = cand_fx ∧ land.face_dy = cand_fy ∧ land.speed = cand_speed ⦄ := by
  unfold step.resolve_one
  simp only [ite_true]
  apply WP.spec_bind hYield
  intro yielded hy
  subst hy
  simp only [Bool.false_eq_true, ite_false]
  h5i_steps

/-- Squared distance on raw coordinates, saturating like `distNat`. -/
def distAt (ax ay bx cy : Nat) : Nat :=
  let dx := absNat ax bx
  let dy := absNat ay cy
  if 2000000 < dx then 2 ^ 64 - 1
  else if 2000000 < dy then 2 ^ 64 - 1
  else dx * dx + dy * dy

theorem absNat_comm (a b : Nat) : absNat a b = absNat b a := by
  unfold absNat
  split <;> split <;> omega

theorem distAt_comm (a b c d : Nat) : distAt a b c d = distAt c d a b := by
  unfold distAt
  simp only [absNat_comm a c, absNat_comm b d]

theorem distNat_distAt (a b c d : U32) : distNat a b c d = distAt a.val b.val c.val d.val := by
  unfold distNat distAt absNat
  rfl

/-- A point strictly inside the collision radius lies in the same tile or a
neighbor tile. The one-tile grid's 3×3 walk is wide enough to see it. -/
theorem within_radius_neighbor_tile (ax ay bx cy : U32)
    (h : distNat ax ay bx cy < 562500) :
    ax.val / 1000 ≤ bx.val / 1000 + 1 ∧ bx.val / 1000 ≤ ax.val / 1000 + 1 ∧
      ay.val / 1000 ≤ cy.val / 1000 + 1 ∧ cy.val / 1000 ≤ ay.val / 1000 + 1 := by
  have gap : ∀ {a b : Nat}, b / 1000 + 2 ≤ a / 1000 → b + 1001 ≤ a := by
    intro a b hgap
    have hmod : b % 1000 < 1000 := Nat.mod_lt b (by decide)
    have hsplit : b = 1000 * (b / 1000) + b % 1000 := (Nat.div_add_mod b 1000).symm
    have ha : 1000 * (a / 1000) ≤ a := Nat.mul_div_le a 1000
    have hmul : 1000 * (b / 1000 + 2) ≤ 1000 * (a / 1000) :=
      Nat.mul_le_mul_left 1000 hgap
    omega
  have along (dx dy : Nat) (hdx : 1001 ≤ dx) :
      562500 ≤ (if 2000000 < dx then 2 ^ 64 - 1
        else if 2000000 < dy then 2 ^ 64 - 1 else dx * dx + dy * dy) := by
    by_cases hx : 2000000 < dx
    · simp only [hx, ite_true]
      decide
    · simp only [hx, ite_false]
      by_cases hy : 2000000 < dy
      · simp only [hy, ite_true]
        decide
      · simp only [hy, ite_false]
        have hsq : 1001 * 1001 ≤ dx * dx := Nat.mul_le_mul hdx hdx
        have hconst : 562500 ≤ 1001 * 1001 := by decide
        exact Nat.le_trans (Nat.le_trans hconst hsq) (Nat.le_add_right _ _)
  have alongY (dx dy : Nat) (hdy : 1001 ≤ dy) :
      562500 ≤ (if 2000000 < dx then 2 ^ 64 - 1
        else if 2000000 < dy then 2 ^ 64 - 1 else dx * dx + dy * dy) := by
    by_cases hx : 2000000 < dx
    · simp only [hx, ite_true]
      decide
    · simp only [hx, ite_false]
      by_cases hy : 2000000 < dy
      · simp only [hy, ite_true]
        decide
      · simp only [hy, ite_false]
        have hsq : 1001 * 1001 ≤ dy * dy := Nat.mul_le_mul hdy hdy
        have hconst : 562500 ≤ 1001 * 1001 := by decide
        exact Nat.le_trans (Nat.le_trans hconst hsq) (Nat.le_add_left _ _)
  refine ⟨?_, ?_, ?_, ?_⟩
  · by_contra hbad
    have hge : bx.val / 1000 + 2 ≤ ax.val / 1000 := by omega
    have hab : bx.val + 1001 ≤ ax.val := gap hge
    have hdx : 1001 ≤ absNat ax.val bx.val := by
      unfold absNat
      have : bx.val ≤ ax.val := by omega
      simp only [this, ite_true]
      omega
    have hdist : 562500 ≤ distNat ax ay bx cy := by
      rw [distNat_distAt]
      unfold distAt
      dsimp
      exact along (absNat ax.val bx.val) (absNat ay.val cy.val) hdx
    omega
  · by_contra hbad
    have hge : ax.val / 1000 + 2 ≤ bx.val / 1000 := by omega
    have hab : ax.val + 1001 ≤ bx.val := gap hge
    have hdx : 1001 ≤ absNat bx.val ax.val := by
      unfold absNat
      have : ax.val ≤ bx.val := by omega
      simp only [this, ite_true]
      omega
    have hdist : 562500 ≤ distNat bx cy ax ay := by
      rw [distNat_distAt]
      unfold distAt
      dsimp
      exact along (absNat bx.val ax.val) (absNat cy.val ay.val) hdx
    have : distNat ax ay bx cy = distNat bx cy ax ay := by
      rw [distNat_distAt, distNat_distAt, distAt_comm]
    omega
  · by_contra hbad
    have hge : cy.val / 1000 + 2 ≤ ay.val / 1000 := by omega
    have hab : cy.val + 1001 ≤ ay.val := gap hge
    have hdy : 1001 ≤ absNat ay.val cy.val := by
      unfold absNat
      have : cy.val ≤ ay.val := by omega
      simp only [this, ite_true]
      omega
    have hdist : 562500 ≤ distNat ax ay bx cy := by
      rw [distNat_distAt]
      unfold distAt
      dsimp
      exact alongY (absNat ax.val bx.val) (absNat ay.val cy.val) hdy
    omega
  · by_contra hbad
    have hge : ay.val / 1000 + 2 ≤ cy.val / 1000 := by omega
    have hab : ay.val + 1001 ≤ cy.val := gap hge
    have hdy : 1001 ≤ absNat cy.val ay.val := by
      unfold absNat
      have : ay.val ≤ cy.val := by omega
      simp only [this, ite_true]
      omega
    have hdist : 562500 ≤ distNat bx cy ax ay := by
      rw [distNat_distAt]
      unfold distAt
      dsimp
      exact alongY (absNat bx.val ax.val) (absNat cy.val ay.val) hdy
    have : distNat ax ay bx cy = distNat bx cy ax ay := by
      rw [distNat_distAt, distNat_distAt, distAt_comm]
    omega

/-- Positions after one two-phase round stay pairwise outside the collision
radius when the previous positions were. `accepted i` means the candidate was
clear of every previous position and of every better-priority proposal.
A rejected agent keeps `(x i, y i)`. An accepted agent was not blocked. -/
theorem two_phase_apart
    (n : Nat) (x y cx cy idOf : Nat → Nat)
    (blocked proposed accepted : Nat → Bool)
    (fx fy : Nat → Nat)
    (hApart : ∀ i j, i < j → j < n → ¬ distAt (x i) (y i) (x j) (y j) < 562500)
    (hOpen : ∀ i, i < n → proposed i = true → blocked i = false)
    (hProp : ∀ i, i < n → proposed i = true →
      ∀ j, j < n → j ≠ i → ¬ distAt (x j) (y j) (cx i) (cy i) < 562500)
    (hAcc : ∀ i, i < n → accepted i = true → proposed i = true ∧
      ∀ j, j < n → j ≠ i → proposed j = true →
        (idOf j < idOf i ∨ idOf j = idOf i ∧ j < i) →
        ¬ distAt (cx j) (cy j) (cx i) (cy i) < 562500)
    (hStay : ∀ i, i < n → accepted i = false → fx i = x i ∧ fy i = y i)
    (hMove : ∀ i, i < n → accepted i = true → fx i = cx i ∧ fy i = cy i) :
    (∀ i j, i < j → j < n → ¬ distAt (fx i) (fy i) (fx j) (fy j) < 562500) ∧
      (∀ i, i < n → accepted i = true → blocked i = false) := by
  have boolFalse {b : Bool} (h : ¬ b = true) : b = false := by
    cases b
    · rfl
    · exact absurd rfl h
  refine ⟨?_, ?_⟩
  · intro i j hij hj
    have hi : i < n := by omega
    by_cases hai : accepted i = true
    · by_cases haj : accepted j = true
      · obtain ⟨hfx, hfy⟩ := hMove i hi hai
        obtain ⟨hgx, hgy⟩ := hMove j hj haj
        have hpropi := (hAcc i hi hai).1
        have hpropj := (hAcc j hj haj).1
        rw [hfx, hfy, hgx, hgy]
        by_cases hlt : idOf i < idOf j
        · exact (hAcc j hj haj).2 i hi (by omega) hpropi (Or.inl hlt)
        · by_cases heq : idOf i = idOf j
          · exact (hAcc j hj haj).2 i hi (by omega) hpropi (Or.inr ⟨heq, hij⟩)
          · have hgt : idOf j < idOf i := by omega
            rw [distAt_comm]
            exact (hAcc i hi hai).2 j hj (by omega) hpropj (Or.inl hgt)
      · obtain ⟨hci, hcyi⟩ := hMove i hi hai
        have hstay := hStay j hj (boolFalse haj)
        have hpropi := (hAcc i hi hai).1
        rw [hci, hcyi, hstay.1, hstay.2, distAt_comm]
        exact hProp i hi hpropi j hj (by omega)
    · by_cases haj : accepted j = true
      · obtain ⟨hcxj, hcyj⟩ := hMove j hj haj
        have hstay := hStay i hi (boolFalse hai)
        have hpropj := (hAcc j hj haj).1
        rw [hstay.1, hstay.2, hcxj, hcyj]
        exact hProp j hj hpropj i hi (by omega)
      · obtain ⟨hxi, hyi⟩ := hStay i hi (boolFalse hai)
        obtain ⟨hxj, hyj⟩ := hStay j hj (boolFalse haj)
        rw [hxi, hyi, hxj, hyj]
        exact hApart i j hij hj
  · intro i hi hai
    exact hOpen i hi (hAcc i hi hai).1

/-- `tick_round` is assumed to succeed and to keep both coordinate slices at
the player count. The body builds the spatial grid and calls `grid_hit` and
`grid_yield`. -/
abbrev RoundOk (s : Snapshot) (moving cand_x cand_y : Slice U32) (ids : Slice U64) : Prop :=
  ∀ (xs ys head next prop_x prop_y prop_on : Slice U32),
    xs.val.length = s.players.val.length →
    ys.val.length = s.players.val.length →
    head.val.length = s.meta.width.val * s.meta.height.val →
    next.val.length = s.players.val.length →
    prop_x.val.length = s.players.val.length →
    prop_y.val.length = s.players.val.length →
    prop_on.val.length = s.players.val.length →
    step.tick_round xs ys moving cand_x cand_y ids
      (alloc.vec.Vec.deref s.terrain.mask) s.meta.width s.meta.height
      head next prop_x prop_y prop_on
      ⦃ r => r.1 = true ∧ r.2.1.val.length = s.players.val.length ∧
        r.2.2.1.val.length = s.players.val.length ⦄

/-- Copying one scratch row per player keeps those rows the same length. -/
theorem tick_players_round_loop0_length
    (v : alloc.vec.Vec Player) (n : Usize)
    (xs ys prop_x prop_y prop_on next : alloc.vec.Vec U32) (i : Usize)
    (hn : n.val = v.val.length) (hi : i.val ≤ n.val)
    (hx : xs.val.length = i.val) (hy : ys.val.length = i.val)
    (hpx : prop_x.val.length = i.val) (hpy : prop_y.val.length = i.val)
    (hpo : prop_on.val.length = i.val) (hnx : next.val.length = i.val) :
    step.tick_players_round_loop0 v n xs ys prop_x prop_y prop_on next i ⦃ r =>
      r.1.val.length = n.val ∧ r.2.1.val.length = n.val ∧
      r.2.2.1.val.length = n.val ∧ r.2.2.2.1.val.length = n.val ∧
      r.2.2.2.2.1.val.length = n.val ∧ r.2.2.2.2.2.val.length = n.val ⦄ := by
  unfold step.tick_players_round_loop0
  apply loop.spec_decr_nat
    (measure := fun (x : alloc.vec.Vec U32 × alloc.vec.Vec U32 × alloc.vec.Vec U32 ×
        alloc.vec.Vec U32 × alloc.vec.Vec U32 × alloc.vec.Vec U32 × Usize) =>
      n.val - x.2.2.2.2.2.2.val)
    (inv := fun x =>
      x.2.2.2.2.2.2.val ≤ n.val ∧
      x.1.val.length = x.2.2.2.2.2.2.val ∧
      x.2.1.val.length = x.2.2.2.2.2.2.val ∧
      x.2.2.1.val.length = x.2.2.2.2.2.2.val ∧
      x.2.2.2.1.val.length = x.2.2.2.2.2.2.val ∧
      x.2.2.2.2.1.val.length = x.2.2.2.2.2.2.val ∧
      x.2.2.2.2.2.1.val.length = x.2.2.2.2.2.2.val)
    (post := fun r =>
      r.1.val.length = n.val ∧ r.2.1.val.length = n.val ∧
      r.2.2.1.val.length = n.val ∧ r.2.2.2.1.val.length = n.val ∧
      r.2.2.2.2.1.val.length = n.val ∧ r.2.2.2.2.2.val.length = n.val)
    (body := fun (xs, ys, prop_x, prop_y, prop_on, next, i) =>
      step.tick_players_round_loop0.body v n xs ys prop_x prop_y prop_on next i)
  · rintro ⟨xs, ys, prop_x, prop_y, prop_on, next, i⟩ ⟨hi, hx, hy, hpx, hpy, hpo, hnx⟩
    simp only at hi hx hy hpx hpy hpo hnx
    unfold step.tick_players_round_loop0.body
    simp only [Vec.index_slice_index]
    split
    · next hlt =>
      have hltN : i.val < n.val := by
        rw [UScalar.lt_equiv] at hlt
        exact hlt
      have hv : i.val < v.val.length := by
        rw [hn] at hltN
        exact hltN
      apply WP.spec_bind (Vec.index_usize_spec v i hv)
      intro p _hp
      have hone : (1#usize).val = 1 := by scalar_tac
      have hvmax := Vec.len_ineq v
      have hxpush : xs.val.length < Usize.max := by rw [hx]; omega
      apply WP.spec_bind (Vec.push_spec xs p.x hxpush)
      intro xs1 hxs1
      have hypush : ys.val.length < Usize.max := by rw [hy]; omega
      apply WP.spec_bind (Vec.push_spec ys p.y hypush)
      intro ys1 hys1
      have hpxpush : prop_x.val.length < Usize.max := by rw [hpx]; omega
      apply WP.spec_bind (Vec.push_spec prop_x (0#u32) hpxpush)
      intro px1 hpx1
      have hpypush : prop_y.val.length < Usize.max := by rw [hpy]; omega
      apply WP.spec_bind (Vec.push_spec prop_y (0#u32) hpypush)
      intro py1 hpy1
      have hpopush : prop_on.val.length < Usize.max := by rw [hpo]; omega
      apply WP.spec_bind (Vec.push_spec prop_on (0#u32) hpopush)
      intro po1 hpo1
      have hnxpush : next.val.length < Usize.max := by rw [hnx]; omega
      apply WP.spec_bind (Vec.push_spec next (0#u32) hnxpush)
      intro nx1 hnx1
      have hadd : i.val + (1#usize).val ≤ Usize.max := by rw [hone]; omega
      apply WP.spec_bind (Usize.add_spec hadd)
      intro i1 hi1
      have hval : i1.val = i.val + 1 := by simpa [hone] using hi1
      simp only [WP.spec_ok]
      refine ⟨⟨?_, ?_, ?_, ?_, ?_, ?_, ?_⟩, ?_⟩
      · omega
      · rw [hxs1, List.length_append, hx, hval]; simp
      · rw [hys1, List.length_append, hy, hval]; simp
      · rw [hpx1, List.length_append, hpx, hval]; simp
      · rw [hpy1, List.length_append, hpy, hval]; simp
      · rw [hpo1, List.length_append, hpo, hval]; simp
      · rw [hnx1, List.length_append, hnx, hval]; simp
      · omega
    · next hge =>
      simp only [WP.spec_ok]
      rw [UScalar.lt_equiv] at hge
      have heqi : i.val = n.val := by omega
      rw [hx, hy, hpx, hpy, hpo, hnx, heqi]
      exact ⟨rfl, rfl, rfl, rfl, rfl, rfl⟩
  · exact ⟨hi, hx, hy, hpx, hpy, hpo, hnx⟩

/-- The cell-head buffer grows by one empty link per tile. -/
theorem tick_players_round_loop1_length
    (cells i : Usize) (head : alloc.vec.Vec U32)
    (hi : i.val ≤ cells.val) (hh : head.val.length = i.val)
    (hcap : cells.val ≤ Usize.max) :
    step.tick_players_round_loop1 i cells head ⦃ h => h.val.length = cells.val ⦄ := by
  unfold step.tick_players_round_loop1
  apply loop.spec_decr_nat
    (measure := fun (x : Usize × alloc.vec.Vec U32) => cells.val - x.1.val)
    (inv := fun x => x.1.val ≤ cells.val ∧ x.2.val.length = x.1.val)
    (post := fun h => h.val.length = cells.val)
    (body := fun (i, head) => step.tick_players_round_loop1.body cells i head)
  · rintro ⟨i, head⟩ ⟨hi, hh⟩
    simp only at hi hh
    unfold step.tick_players_round_loop1.body
    dsimp
    split
    · next hlt =>
      have hltN : i.val < cells.val := by
        rw [UScalar.lt_equiv] at hlt
        exact hlt
      have hpush : head.val.length < Usize.max := by rw [hh]; omega
      apply WP.spec_bind (Vec.push_spec head (0#u32) hpush)
      intro head1 hhead1
      have hone : (1#usize).val = 1 := by scalar_tac
      have hadd : i.val + (1#usize).val ≤ Usize.max := by rw [hone]; omega
      apply WP.spec_bind (Usize.add_spec hadd)
      intro i1 hi1
      have hval : i1.val = i.val + 1 := by simpa [hone] using hi1
      simp only [WP.spec_ok]
      refine ⟨⟨?_, ?_⟩, ?_⟩
      · omega
      · rw [hhead1, List.length_append, hh, hval]; simp
      · omega
    · next hge =>
      simp only [WP.spec_ok]
      rw [UScalar.lt_equiv] at hge
      have heq : i.val = cells.val := by omega
      rw [hh]
      exact heq
  · exact ⟨hi, hh⟩

/-- Writing the landed coordinates back changes `x` and `y` only. -/
theorem tick_players_round_loop2_roster
    (n : Usize) (xs ys : alloc.vec.Vec U32) (v0 v : alloc.vec.Vec Player) (i : Usize)
    (hn : n.val = v0.val.length)
    (hxs : n.val ≤ xs.val.length) (hys : n.val ≤ ys.val.length)
    (hlen : v.val.length = v0.val.length) (hi : i.val ≤ n.val)
    (hids : idsOwners v v0) :
    step.tick_players_round_loop2 v n xs ys i ⦃ v' =>
      v'.val.length = v0.val.length ∧ idsOwners v' v0 ⦄ := by
  unfold step.tick_players_round_loop2
  apply loop.spec_decr_nat
    (measure := fun (x : alloc.vec.Vec Player × Usize) => n.val - x.2.val)
    (inv := fun x => x.2.val ≤ n.val ∧ x.1.val.length = v0.val.length ∧ idsOwners x.1 v0)
    (post := fun v' => v'.val.length = v0.val.length ∧ idsOwners v' v0)
    (body := fun (v, i) => step.tick_players_round_loop2.body n xs ys v i)
  · rintro ⟨v, i⟩ ⟨hi, hlen, hids⟩
    simp only at hi hlen hids
    have _ := hxs
    have _ := hys
    unfold step.tick_players_round_loop2.body
    simp only [Vec.index_slice_index, Vec.index_mut_slice_index]
    split
    · next hlt =>
      have hltN : i.val < n.val := by
        rw [UScalar.lt_equiv] at hlt
        exact hlt
      have hvi : i.val < v.val.length := by
        rw [hlen, ← hn]
        exact hltN
      split
      · next hxsLt =>
        have hxi : i.val < xs.val.length := by
          rw [UScalar.lt_equiv] at hxsLt
          simpa [Vec.len_val, Vec.length] using hxsLt
        split
        · next hysLt =>
          have hyi : i.val < ys.val.length := by
            rw [UScalar.lt_equiv] at hysLt
            simpa [Vec.len_val, Vec.length] using hysLt
          apply WP.spec_bind (Vec.index_usize_spec xs i hxi)
          intro xv _hxv
          apply WP.spec_bind (Vec.index_mut_usize_spec v i hvi)
          intro got hgot
          simp only [WP.uncurry'_eq] at hgot
          rcases got with ⟨p, back⟩
          dsimp at hgot
          apply WP.spec_bind (Vec.index_usize_spec ys i hyi)
          intro yv _hyv
          have hp : p = v.val[i.val] := hgot.1
          have hback : back = v.set i := hgot.2
          rw [hback]
          have hxPlayer : ({ p with x := xv }).id = (v.val[i.val]'hvi).id ∧
              ({ p with x := xv }).owner = (v.val[i.val]'hvi).owner := by
            rw [hp]
            exact ⟨rfl, rfl⟩
          have hlen1 := roster_set v0 v i { p with x := xv } hlen hvi hxPlayer hids
          have hiSet : i.val < ((v.set i) { p with x := xv }).val.length := by
            rw [hlen1.1, ← hlen]
            exact hvi
          apply WP.spec_bind
            (Vec.index_mut_usize_spec ((v.set i) { p with x := xv }) i hiSet)
          intro got1 hgot1
          simp only [WP.uncurry'_eq] at hgot1
          rcases got1 with ⟨p1, back1⟩
          dsimp at hgot1
          have hone : (1#usize).val = 1 := by scalar_tac
          have hadd : i.val + (1#usize).val ≤ Usize.max := by
            have hvmax := Vec.len_ineq v0
            rw [hone]
            omega
          apply WP.spec_bind (Usize.add_spec hadd)
          intro i1 hi1
          have hstep : i1.val = i.val + 1 := by
            simpa [hone] using hi1
          have hp1 : p1 = ((v.set i) { p with x := xv }).val[i.val] := hgot1.1
          have hback1 : back1 = ((v.set i) { p with x := xv }).set i := hgot1.2
          have hyPlayer : ({ p1 with y := yv }).id =
                (((v.set i) { p with x := xv }).val[i.val]'hiSet).id ∧
              ({ p1 with y := yv }).owner =
                (((v.set i) { p with x := xv }).val[i.val]'hiSet).owner := by
            rw [hp1]
            exact ⟨rfl, rfl⟩
          have hlen2 := roster_set v0 ((v.set i) { p with x := xv }) i
            { p1 with y := yv } hlen1.1 hiSet hyPlayer hlen1.2
          simp only [WP.spec_ok, hback1]
          refine ⟨⟨?_, hlen2.1, hlen2.2⟩, ?_⟩
          · omega
          · omega
        · simp only [WP.spec_ok]
          exact ⟨hlen, hids⟩
      · simp only [WP.spec_ok]
        exact ⟨hlen, hids⟩
    · simp only [WP.spec_ok]
      exact ⟨hlen, hids⟩
  · exact ⟨hi, hlen, hids⟩

/-- One two-phase pass keeps every field except player coordinates, and it
keeps ids and owners. `hCells` is the map area, which fits in one `usize`. -/
theorem tick_players_round_keeps_roster
    (s : Snapshot) (moving cand_x cand_y : Slice U32) (ids : Slice U64)
    (hStep : RoundOk s moving cand_x cand_y ids)
    (hCells : s.meta.width.val * s.meta.height.val ≤ Usize.max) :
    step.tick_players_round s moving cand_x cand_y ids ⦃ s' =>
      s'.meta = s.meta ∧ s'.terrain = s.terrain ∧ s'.paths = s.paths ∧
      s'.conversations = s.conversations ∧ s'.members = s.members ∧
      s'.agents = s.agents ∧ s'.memories = s.memories ∧ s'.messages = s.messages ∧
      s'.players.val.length = s.players.val.length ∧ idsOwners s'.players s.players ⦄ := by
  unfold step.tick_players_round
  set n := alloc.vec.Vec.len s.players with hn
  have hnval : n.val = s.players.val.length := by simp [hn]
  have hempty : (alloc.vec.Vec.new U32).val.length = 0 := by simp [alloc.vec.Vec.new, Vec.from_val]
  have hzero : (0#usize).val = 0 := by scalar_tac
  apply WP.spec_bind (tick_players_round_loop0_length s.players n
      (alloc.vec.Vec.new U32) (alloc.vec.Vec.new U32) (alloc.vec.Vec.new U32)
      (alloc.vec.Vec.new U32) (alloc.vec.Vec.new U32) (alloc.vec.Vec.new U32) 0#usize
      hnval (by rw [hzero]; exact Nat.zero_le _)
      (by rw [hempty, hzero]) (by rw [hempty, hzero])
      (by rw [hempty, hzero]) (by rw [hempty, hzero])
      (by rw [hempty, hzero]) (by rw [hempty, hzero]))
  intro copied hcopied
  obtain ⟨xs, ys, prop_x, prop_y, prop_on, next⟩ := copied
  obtain ⟨hxs, hys, hpx, hpy, hpo, hnx⟩ := hcopied
  simp only [lift, bind_ok]
  have hmul : (UScalar.cast .Usize s.meta.width).val * (UScalar.cast .Usize s.meta.height).val ≤ Usize.max := by
    simp only [U32.cast_Usize_val_eq]
    exact hCells
  apply WP.spec_bind (Usize.mul_spec hmul)
  intro cells hcells
  have hcells0 : (0#usize).val ≤ cells.val := by rw [hzero]; exact Nat.zero_le _
  have hcap : cells.val ≤ Usize.max := by
    rw [hcells]
    simpa [U32.cast_Usize_val_eq] using hCells
  apply WP.spec_bind (tick_players_round_loop1_length cells 0#usize (alloc.vec.Vec.new U32) hcells0
      (by rw [hempty, hzero]) hcap)
  intro head hhead
  simp only [alloc.vec.Vec.deref_mut]
  have hxsL : xs.slice.val.length = s.players.val.length := by simpa [Vec.val, hnval] using hxs
  have hysL : ys.slice.val.length = s.players.val.length := by simpa [Vec.val, hnval] using hys
  have hpxL : prop_x.slice.val.length = s.players.val.length := by simpa [Vec.val, hnval] using hpx
  have hpyL : prop_y.slice.val.length = s.players.val.length := by simpa [Vec.val, hnval] using hpy
  have hpoL : prop_on.slice.val.length = s.players.val.length := by simpa [Vec.val, hnval] using hpo
  have hnxL : next.slice.val.length = s.players.val.length := by simpa [Vec.val, hnval] using hnx
  have hheadL : head.slice.val.length = s.meta.width.val * s.meta.height.val := by
    simpa [Vec.val, hcells, U32.cast_Usize_val_eq] using hhead
  apply WP.spec_bind (hStep xs.slice ys.slice head.slice next.slice prop_x.slice prop_y.slice prop_on.slice
      hxsL hysL hheadL hnxL hpxL hpyL hpoL)
  intro moved hmoved
  obtain ⟨_, xs', ys', _, _, _, _, _⟩ := moved
  obtain ⟨_, hxs', hys'⟩ := hmoved
  have hxlen : ({ slice := xs' } : alloc.vec.Vec U32).val.length = s.players.val.length := by
    simpa [Vec.val] using hxs'
  have hylen : ({ slice := ys' } : alloc.vec.Vec U32).val.length = s.players.val.length := by
    simpa [Vec.val] using hys'
  apply WP.spec_bind (tick_players_round_loop2_roster n { slice := xs' } { slice := ys' }
      s.players s.players 0#usize hnval
      (Nat.le_of_eq (by rw [hxlen, hnval]))
      (Nat.le_of_eq (by rw [hylen, hnval]))
      rfl (by rw [hzero]; exact Nat.zero_le _)
      (by intro j hj hj0; exact ⟨rfl, rfl⟩))
  intro v hv
  simp only [WP.spec_ok]
  obtain ⟨hlen, hids⟩ := hv
  refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, hlen, hids⟩
  all_goals first | rfl | trivial

/-- A two-phase pass that keeps members keeps the exclusivity verdict. -/
theorem tick_players_round_preserves_exclusive
    (s : Snapshot) (moving cand_x cand_y : Slice U32) (ids : Slice U64)
    (hStep : RoundOk s moving cand_x cand_y ids)
    (hCells : s.meta.width.val * s.meta.height.val ≤ Usize.max)
    (hEx : rules.exclusive s ⦃ b => b = true ⦄) :
    step.tick_players_round s moving cand_x cand_y ids ⦃ s' =>
      rules.exclusive s' ⦃ b => b = true ⦄ ⦄ := by
  apply WP.spec_mono (tick_players_round_keeps_roster s moving cand_x cand_y ids hStep hCells)
  intro s' hs
  rw [exclusive_of_members s' s hs.2.2.2.2.1]
  exact hEx

/-- A two-phase pass that keeps ids, members, and conversations keeps the
membership verdict. -/
theorem tick_players_round_preserves_members
    (s : Snapshot) (moving cand_x cand_y : Slice U32) (ids : Slice U64)
    (hStep : RoundOk s moving cand_x cand_y ids)
    (hCells : s.meta.width.val * s.meta.height.val ≤ Usize.max)
    (hMem : rules.members_consistent s ⦃ b => b = true ⦄) :
    step.tick_players_round s moving cand_x cand_y ids ⦃ s' =>
      rules.members_consistent s' ⦃ b => b = true ⦄ ⦄ := by
  apply WP.spec_mono (tick_players_round_keeps_roster s moving cand_x cand_y ids hStep hCells)
  intro s' hs
  have hlen : s'.players.val.length = s.players.val.length := hs.2.2.2.2.2.2.2.2.1
  have hids : idsOwners s'.players s.players := hs.2.2.2.2.2.2.2.2.2
  have hP : rules.player_index s'.players = rules.player_index s.players := by
    funext id
    exact player_index_of_ids s'.players s.players id hlen
      (by intro j hj; exact (hids j hj (by omega)).1)
  rw [members_consistent_of_roster s' s hs.2.2.2.1 hs.2.2.2.2.1 hP]
  exact hMem

/-- A two-phase pass that keeps ids and owners resolves `own_player` the same
way. -/
theorem tick_players_round_preserves_own_player
    (s : Snapshot) (moving cand_x cand_y : Slice U32) (ids : Slice U64)
    (hStep : RoundOk s moving cand_x cand_y ids)
    (hCells : s.meta.width.val * s.meta.height.val ≤ Usize.max)
    (owner : U64) :
    step.tick_players_round s moving cand_x cand_y ids ⦃ s' =>
      rules.own_player s' owner = rules.own_player s owner ∧
      rules.own_player s' owner ⦃ r => ∀ id, r = .Ok id →
        ∃ n, ∃ hn : n < s'.players.val.length,
          (s'.players.val[n]'hn).id = id ∧ (s'.players.val[n]'hn).owner = owner ⦄ ⦄ := by
  apply WP.spec_mono (tick_players_round_keeps_roster s moving cand_x cand_y ids hStep hCells)
  intro s' hs
  have hlen : s'.players.val.length = s.players.val.length := hs.2.2.2.2.2.2.2.2.1
  have hids : idsOwners s'.players s.players := hs.2.2.2.2.2.2.2.2.2
  refine ⟨?_, ?_⟩
  · exact own_player_of_ids s' s owner hlen
      (by intro j hj; exact hids j hj (by omega))
  · exact motion_isolated_for_player s' owner

end ai_town_kernel

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

end ai_town_kernel

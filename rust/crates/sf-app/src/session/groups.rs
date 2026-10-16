//! Sample groups and group-owned gates.
//!
//! The model follows what FlowJo shows in its menus and dialogs (names and behaviour taken from its labels, no code):
//! "All Samples" always exists and cannot be edited or deleted; other groups hold chosen samples, are made from a
//! selection or from the values of a keyword, and have a colour and a *synchronized* option. A gate applied to a
//! group is **owned by the group**: it is the same gate in every member sample. Editing it in one sample leaves
//! that sample's copy *overridden* (shown as edited) unless the group is synchronized, in which case the edit
//! reaches every member. A group-owned gate cannot simply be deleted in one sample; it is deleted from the group
//! (entirely, keeping one copy in the first sample, or handing the copies to the samples).

use super::*;
use sf_gating::workspace::{gates_from_json, gates_to_json};
use sf_gating::Gate;

pub const ALL_SAMPLES: &str = "All Samples";

#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    /// Sample names (so a group survives reloading the files in another order).
    pub members: Vec<String>,
    pub color: [u8; 3],
    pub synced: bool,
}

/// A gate shared by a group: its population path and the group's version of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub path: Vec<String>,
    pub group: String,
    pub gate: Gate,
}

/// Where a newly drawn gate goes.
#[derive(Debug, Clone, PartialEq)]
pub enum DrawTarget {
    /// Only the sample it was drawn in.
    ThisSample,
    /// Owned by this group ("All Samples" or a named group), so it appears in every member.
    Group(String),
}

impl Default for DrawTarget {
    fn default() -> Self {
        DrawTarget::ThisSample
    }
}

impl DrawTarget {
    /// What the application starts with: a gate drawn in one sample appears in every sample.
    pub fn app_default() -> DrawTarget {
        DrawTarget::Group(ALL_SAMPLES.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupDelete {
    /// Remove every copy, except where a sample has overridden it.
    Entirely,
    /// Remove it from every sample but the first member.
    KeepInFirstSample,
    /// Remove the group's ownership and leave the gate in each sample as its own.
    ToSamples,
}

/// How a sample's gate relates to its owning group.
#[derive(Debug, Clone, PartialEq)]
pub struct GateOwner {
    pub group: String,
    /// This sample's version differs from the group's.
    pub edited: bool,
    pub color: Option<[u8; 3]>,
}

fn is_geometric(g: &Gate) -> bool {
    !matches!(g.kind, GateKind::Indexed(_))
}

impl Session {
    // ---- membership ------------------------------------------------------------------------------

    pub fn group(&self, name: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.name == name)
    }

    /// Indices of the samples in a group ("All Samples" is every loaded sample).
    pub fn member_indices(&self, group: &str) -> Vec<usize> {
        if group == ALL_SAMPLES {
            return (0..self.samples.len()).collect();
        }
        let Some(g) = self.group(group) else { return Vec::new() };
        (0..self.samples.len()).filter(|&i| g.members.contains(&self.samples[i].name)).collect()
    }

    pub fn groups_of(&self, sample: usize) -> Vec<String> {
        let name = &self.samples[sample].name;
        self.groups.iter().filter(|g| g.members.contains(name)).map(|g| g.name.clone()).collect()
    }

    fn check_group_name(&self, name: &str, except: Option<&str>) -> Result<(), String> {
        let n = name.trim();
        if n.is_empty() {
            return Err("A group needs a name".into());
        }
        if n == ALL_SAMPLES {
            return Err(format!("'{ALL_SAMPLES}' is the built-in group; choose another name"));
        }
        if self.groups.iter().any(|g| g.name == n && Some(g.name.as_str()) != except) {
            return Err(format!("A group named '{n}' already exists"));
        }
        Ok(())
    }

    /// Make a group of the given samples.
    pub fn create_group(&mut self, name: &str, members: &[usize]) -> Result<(), String> {
        self.check_group_name(name, None)?;
        if members.is_empty() {
            return Err("Select at least one sample for the group".into());
        }
        let mut names: Vec<String> = members.iter().filter_map(|&i| self.samples.get(i).map(|s| s.name.clone())).collect();
        names.dedup();
        self.groups.push(Group { name: name.trim().to_string(), members: names, color: [0x1F, 0x6F, 0xEB], synced: false });
        Ok(())
    }

    /// One group per distinct value of an FCS keyword ("Create group from keyword value"), named after the value.
    /// A group that already has that name gets the new membership. Samples without the keyword are left out.
    pub fn groups_from_keyword(&mut self, keyword: &str) -> Result<Vec<String>, String> {
        let key = keyword.trim().trim_start_matches('$').to_uppercase();
        if key.is_empty() {
            return Err("Choose a keyword".into());
        }
        let mut by_value: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for s in &self.samples {
            let v = s.keywords.iter().find(|(k, _)| k.trim().trim_start_matches('$').to_uppercase() == key).map(|(_, v)| v.trim().to_string()).filter(|v| !v.is_empty());
            if let Some(v) = v {
                by_value.entry(v).or_default().push(s.name.clone());
            }
        }
        if by_value.is_empty() {
            return Err(format!("No loaded sample has a value for the keyword '{keyword}'"));
        }
        let mut made = Vec::new();
        for (value, members) in by_value {
            if value == ALL_SAMPLES {
                continue;
            }
            match self.groups.iter_mut().find(|g| g.name == value) {
                Some(g) => g.members = members,
                None => self.groups.push(Group { name: value.clone(), members, color: [0x1F, 0x6F, 0xEB], synced: false }),
            }
            made.push(value);
        }
        Ok(made)
    }

    /// Change a group's name, members, colour and synchronization. The gates it owns follow a rename.
    pub fn modify_group(&mut self, old: &str, new_name: &str, members: &[usize], color: [u8; 3], synced: bool) -> Result<(), String> {
        if old == ALL_SAMPLES {
            return Err(format!("The '{ALL_SAMPLES}' group cannot be edited"));
        }
        self.check_group_name(new_name, Some(old))?;
        if members.is_empty() {
            return Err("A group needs at least one sample".into());
        }
        let names: Vec<String> = members.iter().filter_map(|&i| self.samples.get(i).map(|s| s.name.clone())).collect();
        let g = self.groups.iter_mut().find(|g| g.name == old).ok_or("No such group")?;
        *g = Group { name: new_name.trim().to_string(), members: names, color, synced };
        let new_name = new_name.trim().to_string();
        for a in &mut self.applied {
            if a.group == old {
                a.group = new_name.clone();
            }
        }
        if let DrawTarget::Group(n) = &mut self.draw_target {
            if n == old {
                *n = new_name;
            }
        }
        Ok(())
    }

    /// Delete a group. Gates it owns are dealt with as `mode` says. Returns the number of gate copies removed.
    pub fn delete_group(&mut self, name: &str, mode: GroupDelete) -> Result<usize, String> {
        if name == ALL_SAMPLES {
            return Err(format!("The '{ALL_SAMPLES}' group cannot be deleted"));
        }
        if self.group(name).is_none() {
            return Err("No such group".into());
        }
        let owned: Vec<Applied> = self.applied.iter().filter(|a| a.group == name).cloned().collect();
        let mut removed = 0;
        for a in owned {
            removed += self.dispose_group_gate(&a, mode);
        }
        self.applied.retain(|a| a.group != name);
        self.groups.retain(|g| g.name != name);
        if self.draw_target == DrawTarget::Group(name.to_string()) {
            self.draw_target = DrawTarget::default();
        }
        Ok(removed)
    }

    // ---- ownership -------------------------------------------------------------------------------

    fn applied_index(&self, sample: usize, path: &[String]) -> Option<usize> {
        self.applied.iter().position(|a| a.path == path && self.member_indices(&a.group).contains(&sample))
    }

    /// The group that owns this sample's gate, and whether the sample has changed it since.
    pub fn gate_owner(&self, sample: usize, node: NodeId) -> Option<GateOwner> {
        let path = self.states.get(sample)?.tree.path(node).ok()?;
        if path.is_empty() {
            return None;
        }
        let a = &self.applied[self.applied_index(sample, &path)?];
        let edited = self.gate_of(sample, node).map_or(true, |g| g != a.gate);
        let color = if a.group == ALL_SAMPLES { None } else { self.group(&a.group).map(|g| g.color) };
        Some(GateOwner { group: a.group.clone(), edited, color })
    }

    fn find_path(&self, sample: usize, path: &[String]) -> Option<NodeId> {
        let p: Vec<&str> = path.iter().map(String::as_str).collect();
        self.states[sample].tree.find(&p)
    }

    /// Give this gate to a group: every member gets the same gate at the same place in its hierarchy (children of
    /// an existing copy are kept), and the group becomes its owner. Returns how many other samples received it.
    pub fn apply_to_group(&mut self, sample: usize, node: NodeId, group: &str) -> Result<usize, String> {
        let path = self.states[sample].tree.path(node).map_err(|e| e.to_string())?;
        if path.is_empty() {
            return Err("Select a population first".into());
        }
        let gate = match self.states[sample].tree.kind(node).map_err(|e| e.to_string())? {
            NodeKind::Gate(g) if is_geometric(g) => g.clone(),
            NodeKind::Gate(_) => return Err("Subsample and plugin populations hold the events of one file and cannot be applied to a group".into()),
            _ => return Err("Only geometric gates can be applied to a group".into()),
        };
        if group != ALL_SAMPLES && self.group(group).is_none() {
            return Err(format!("There is no group '{group}'"));
        }
        let members = self.member_indices(group);
        if !members.contains(&sample) {
            return Err(format!("'{}' is not in the group '{group}'", self.samples[sample].name));
        }
        let mut done = 0;
        let name = path.last().unwrap().clone();
        for other in members {
            if other == sample {
                continue;
            }
            let parent_path: Vec<&str> = path[..path.len() - 1].iter().map(String::as_str).collect();
            let parent = if parent_path.is_empty() { Some(GateTree::ROOT) } else { self.states[other].tree.find(&parent_path) };
            let Some(p) = parent else {
                self.log.push(format!("{}: skipped — it has no '{}' to put the gate under", self.samples[other].name, parent_path.join(" > ")));
                continue;
            };
            self.snapshot(other);
            let existing = self.find_path(other, &path).filter(|&n| matches!(self.states[other].tree.kind(n), Ok(NodeKind::Gate(g)) if is_geometric(g)));
            let r = match existing {
                Some(n) => self.states[other].tree.reset_gate(n, gate.clone()).map(|_| ()),
                None => self.states[other].tree.replace_gate(p, &name, gate.clone()).map(|_| ()),
            };
            if let Err(e) = r {
                self.discard_snapshot(other);
                return Err(format!("{}: {e}", self.samples[other].name));
            }
            self.touch(other);
            done += 1;
        }
        self.applied.retain(|a| a.path != path);
        // a group of one sample shares nothing, so there is nothing to own
        if self.member_indices(group).len() > 1 {
            self.applied.push(Applied { path, group: group.to_string(), gate });
        }
        Ok(done)
    }

    /// Hand the sample's gate back to the samples: the group stops owning it and every copy stays as it is.
    pub fn release_gate(&mut self, sample: usize, node: NodeId) -> Result<(), String> {
        let path = self.states[sample].tree.path(node).map_err(|e| e.to_string())?;
        let i = self.applied_index(sample, &path).ok_or("This gate is not owned by a group")?;
        self.applied.remove(i);
        Ok(())
    }

    /// Put the group's version back in this sample (discarding the sample's changes to it).
    pub fn revert_to_group(&mut self, sample: usize, node: NodeId) -> Result<usize, String> {
        let path = self.states[sample].tree.path(node).map_err(|e| e.to_string())?;
        let i = self.applied_index(sample, &path).ok_or("This gate is not owned by a group")?;
        let g = self.applied[i].gate.clone();
        self.snapshot(sample);
        if let Err(e) = self.states[sample].tree.reset_gate(node, g) {
            self.discard_snapshot(sample);
            return Err(e.to_string());
        }
        self.touch(sample);
        Ok(self.count(sample, node))
    }

    /// Make this sample's version the group's and give it to every member.
    pub fn update_group_from(&mut self, sample: usize, node: NodeId) -> Result<usize, String> {
        let o = self.gate_owner(sample, node).ok_or("This gate is not owned by a group")?;
        self.apply_to_group(sample, node, &o.group)
    }

    /// Everything the group owns that is no longer in any member (after deletes) is forgotten.
    pub(super) fn prune_applied(&mut self) {
        let keep: Vec<bool> = self.applied.iter().map(|a| self.member_indices(&a.group).into_iter().any(|s| self.find_path(s, &a.path).is_some())).collect();
        let mut it = keep.into_iter();
        self.applied.retain(|_| it.next().unwrap_or(true));
    }

    /// Remove one owned gate from the members as `mode` says; returns the copies removed.
    fn dispose_group_gate(&mut self, a: &Applied, mode: GroupDelete) -> usize {
        if mode == GroupDelete::ToSamples {
            return 0;
        }
        let members = self.member_indices(&a.group);
        let first = members.first().copied();
        let mut removed = 0;
        for s in members {
            let Some(n) = self.find_path(s, &a.path) else { continue };
            let overridden = self.gate_of(s, n).map_or(false, |g| g != a.gate);
            let keep = match mode {
                GroupDelete::Entirely => overridden,
                GroupDelete::KeepInFirstSample => Some(s) == first,
                GroupDelete::ToSamples => true,
            };
            if !keep && self.delete_unchecked(s, n).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    /// Delete a group-owned gate from its group (FlowJo's "Delete Group Analysis?"). Returns the copies removed.
    pub fn delete_from_group(&mut self, sample: usize, node: NodeId, mode: GroupDelete) -> Result<usize, String> {
        let path = self.states[sample].tree.path(node).map_err(|e| e.to_string())?;
        let i = self.applied_index(sample, &path).ok_or("This gate is not owned by a group")?;
        let a = self.applied.remove(i);
        let n = self.dispose_group_gate(&a, mode);
        self.prune_applied();
        Ok(n)
    }

    /// A synchronized group's gate moved in one sample: the others follow and the group's version becomes this one.
    pub(super) fn propagate_if_synced(&mut self, sample: usize, node: NodeId) {
        let Some(path) = self.states[sample].tree.path(node).ok() else { return };
        let Some(i) = self.applied_index(sample, &path) else { return };
        let group = self.applied[i].group.clone();
        let synced = if group == ALL_SAMPLES { self.all_synced } else { self.group(&group).map_or(false, |g| g.synced) };
        let Some(gate) = self.gate_of(sample, node) else { return };
        if !synced {
            return;
        }
        for other in self.member_indices(&group) {
            if other == sample {
                continue;
            }
            if let Some(n) = self.find_path(other, &path) {
                self.snapshot(other);
                if self.states[other].tree.reset_gate(n, gate.clone()).is_ok() {
                    self.touch(other);
                } else {
                    self.discard_snapshot(other);
                }
            }
        }
        self.applied[i].gate = gate;
    }

    /// Rename a group-owned gate in every member ("Rename for all?").
    pub(super) fn rename_group_owned(&mut self, sample: usize, node: NodeId, new_name: &str) -> Result<(), String> {
        let path = self.states[sample].tree.path(node).map_err(|e| e.to_string())?;
        let i = self.applied_index(sample, &path).ok_or("not group owned")?;
        let group = self.applied[i].group.clone();
        if new_name.trim().is_empty() {
            return Err("a population needs a name".into());
        }
        let members = self.member_indices(&group);
        // check first so a clash in any sample leaves everything as it was
        for &s in &members {
            if let Some(n) = self.find_path(s, &path) {
                let t = &self.states[s].tree;
                let parent = t.parent(n).ok().flatten().ok_or("the root cannot be renamed")?;
                if t.children(parent).map_err(|e| e.to_string())?.iter().any(|&c| c != n && t.name(c).map_or(false, |x| x == new_name)) {
                    return Err(format!("'{new_name}' already exists at this level in {}", self.samples[s].name));
                }
            }
        }
        for &s in &members {
            if let Some(n) = self.find_path(s, &path) {
                self.snapshot(s);
                self.states[s].tree.rename(n, new_name).map_err(|e| e.to_string())?;
                self.touch(s);
            }
        }
        let depth = path.len();
        for a in &mut self.applied {
            if a.path.len() >= depth && a.path[..depth] == path[..] {
                a.path[depth - 1] = new_name.to_string();
            }
        }
        Ok(())
    }

    /// Apply a freshly drawn gate to the draw target (a group), when there is one. Returns the samples reached.
    pub(super) fn apply_new_gate(&mut self, sample: usize, node: NodeId) -> usize {
        let DrawTarget::Group(g) = self.draw_target.clone() else { return 0 };
        match self.apply_to_group(sample, node, &g) {
            Ok(n) => n,
            Err(e) => {
                self.log.push(format!("Gate kept in this sample only: {e}"));
                0
            }
        }
    }

    // ---- saving ----------------------------------------------------------------------------------

    fn gate_to_json(g: &Gate) -> Json {
        let mut t = GateTree::new("r");
        let _ = t.add_gate(GateTree::ROOT, "g", g.clone());
        gates_to_json(&t, &HashMap::new())
    }

    fn gate_from_json(j: &Json) -> Option<Gate> {
        let (sg, _) = gates_from_json(j, "r").ok()?;
        match sg.tree.kind(*sg.tree.children(GateTree::ROOT).ok()?.first()?) {
            Ok(NodeKind::Gate(g)) => Some(g.clone()),
            _ => None,
        }
    }

    pub(super) fn groups_to_json(&self) -> Json {
        let groups = self
            .groups
            .iter()
            .map(|g| {
                Json::obj()
                    .with("name", Json::Str(g.name.clone()))
                    .with("members", Json::Arr(g.members.iter().map(|m| Json::Str(m.clone())).collect()))
                    .with("color", Json::Arr(g.color.iter().map(|c| Json::num(*c as f64)).collect()))
                    .with("synced", Json::Bool(g.synced))
            })
            .collect();
        let applied = self
            .applied
            .iter()
            .map(|a| Json::obj().with("path", Json::Arr(a.path.iter().map(|p| Json::Str(p.clone())).collect())).with("group", Json::Str(a.group.clone())).with("gate", Self::gate_to_json(&a.gate)))
            .collect();
        let target = match &self.draw_target {
            DrawTarget::ThisSample => Json::Null,
            DrawTarget::Group(g) => Json::Str(g.clone()),
        };
        Json::obj().with("groups", Json::Arr(groups)).with("applied", Json::Arr(applied)).with("draw_target", target).with("all_synced", Json::Bool(self.all_synced))
    }

    /// Read groups back. Problems are returned as notes; whatever could be read is kept.
    pub(super) fn groups_from_json(&mut self, doc: &Json) -> Vec<String> {
        let mut notes = Vec::new();
        self.groups.clear();
        self.applied.clear();
        self.all_synced = false;
        let strs = |j: &Json| -> Vec<String> { j.as_arr().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
        if let Some(a) = doc.get("groups").and_then(Json::as_arr) {
            for g in a {
                let Some(name) = g.get("name").and_then(Json::as_str).filter(|n| !n.is_empty() && *n != ALL_SAMPLES) else {
                    notes.push("a saved group without a usable name was ignored".into());
                    continue;
                };
                let c: Vec<u8> = g.get("color").and_then(Json::as_arr).map(|a| a.iter().filter_map(Json::as_f64).map(|v| v.clamp(0.0, 255.0) as u8).collect()).unwrap_or_default();
                let members = g.get("members").map(strs).unwrap_or_default();
                for m in &members {
                    if !self.samples.iter().any(|s| &s.name == m) {
                        notes.push(format!("group '{name}' lists '{m}', which is not loaded"));
                    }
                }
                self.groups.push(Group { name: name.to_string(), members, color: if c.len() == 3 { [c[0], c[1], c[2]] } else { [0x1F, 0x6F, 0xEB] }, synced: g.get("synced").and_then(Json::as_bool).unwrap_or(false) });
            }
        }
        if let Some(a) = doc.get("applied").and_then(Json::as_arr) {
            for x in a {
                let path = x.get("path").map(strs).unwrap_or_default();
                let group = x.get("group").and_then(Json::as_str).unwrap_or("").to_string();
                let gate = x.get("gate").and_then(Self::gate_from_json);
                match gate {
                    Some(gate) if !path.is_empty() && (group == ALL_SAMPLES || self.group(&group).is_some()) => self.applied.push(Applied { path, group, gate }),
                    _ => notes.push("a saved group gate could not be read and is no longer shared".into()),
                }
            }
        }
        // a file from before groups existed has no draw target: whatever the session had stays
        match doc.get("draw_target") {
            Some(Json::Null) => self.draw_target = DrawTarget::ThisSample,
            Some(Json::Str(g)) if g == ALL_SAMPLES || self.group(g).is_some() => self.draw_target = DrawTarget::Group(g.clone()),
            _ => {}
        }
        self.all_synced = doc.get("all_synced").and_then(Json::as_bool).unwrap_or(false);
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_gating::GateKind;

    fn sess(n: usize) -> Session {
        let mut s = Session::new();
        for i in 0..n {
            let data: Vec<f32> = (0..2000).flat_map(|k| [((k * 37 + i * 11) % 1000) as f32, ((k * 11 + i * 7) % 500) as f32]).collect();
            let mut smp = crate::sample::LoadedSample::synthetic(&format!("s{i}.fcs"), &["FSC-A", "SSC-A"], vec![1000.0, 1000.0], data);
            smp.keywords.insert("$FIL".into(), format!("s{i}.fcs"));
            smp.keywords.insert("CONDITION".into(), if i % 2 == 0 { "Control".into() } else { "Treated".into() });
            s.add_sample(smp);
        }
        s
    }

    fn rect(x0: f64, x1: f64) -> Gate {
        Gate::new(GateKind::Rectangle, "FSC-A", Some("SSC-A"), vec![x0, x1], vec![0.0, 300.0]).unwrap()
    }

    fn draw(s: &mut Session, si: usize, name: &str, g: Gate) -> NodeId {
        s.states[si].tree.add_gate(GateTree::ROOT, name, g).unwrap()
    }

    #[test]
    fn groups_are_made_from_a_selection_or_a_keyword_and_all_samples_is_protected() {
        let mut s = sess(4);
        s.create_group("Pair", &[0, 2]).unwrap();
        assert_eq!(s.member_indices("Pair"), vec![0, 2]);
        assert_eq!(s.member_indices(ALL_SAMPLES), vec![0, 1, 2, 3]);
        assert!(s.create_group("Pair", &[1]).unwrap_err().contains("already"));
        assert!(s.create_group(ALL_SAMPLES, &[1]).is_err());
        assert!(s.create_group("  ", &[1]).is_err());
        assert!(s.create_group("Empty", &[]).is_err());
        let made = s.groups_from_keyword("condition").unwrap();
        assert_eq!(made, vec!["Control".to_string(), "Treated".to_string()]);
        assert_eq!(s.member_indices("Control"), vec![0, 2]);
        assert_eq!(s.member_indices("Treated"), vec![1, 3]);
        assert!(s.groups_from_keyword("NOPE").is_err());
        assert_eq!(s.groups_of(1), vec!["Treated".to_string()]);
        assert!(s.modify_group(ALL_SAMPLES, "x", &[0], [0; 3], false).unwrap_err().contains("cannot be edited"));
        assert!(s.delete_group(ALL_SAMPLES, GroupDelete::Entirely).unwrap_err().contains("cannot be deleted"));
        s.modify_group("Pair", "Duo", &[0, 1, 2], [1, 2, 3], true).unwrap();
        assert_eq!(s.member_indices("Duo"), vec![0, 1, 2]);
        assert!(s.group("Duo").unwrap().synced);
        assert!(s.modify_group("Duo", "Control", &[0], [0; 3], false).is_err(), "no duplicate names");
    }

    #[test]
    fn applying_a_gate_to_a_group_reaches_exactly_its_members_and_keeps_children() {
        let mut s = sess(4);
        s.create_group("Pair", &[0, 2]).unwrap();
        let g = draw(&mut s, 0, "Lymph", rect(100.0, 600.0));
        // a child gate already under another member's copy must survive re-applying
        let other = draw(&mut s, 2, "Lymph", rect(0.0, 50.0));
        let child = s.states[2].tree.add_gate(other, "Kid", rect(0.0, 20.0)).unwrap();
        assert_eq!(s.apply_to_group(0, g, "Pair").unwrap(), 1);
        assert!(s.find_path(1, &["Lymph".into()]).is_none() && s.find_path(3, &["Lymph".into()]).is_none(), "non-members get nothing");
        let n2 = s.find_path(2, &["Lymph".into()]).unwrap();
        assert_eq!(s.gate_of(2, n2).unwrap(), rect(100.0, 600.0));
        assert_eq!(s.find_path(2, &["Lymph".into(), "Kid".into()]), Some(child), "the child kept its place");
        assert_eq!(s.gate_owner(0, g).unwrap().group, "Pair");
        assert_eq!(s.gate_owner(2, n2).unwrap(), GateOwner { group: "Pair".into(), edited: false, color: Some([0x1F, 0x6F, 0xEB]) });
        assert!(s.gate_owner(1, 1).is_none());
        // a sample outside the group cannot apply to it
        let g1 = draw(&mut s, 1, "Other", rect(1.0, 2.0));
        assert!(s.apply_to_group(1, g1, "Pair").unwrap_err().contains("not in the group"));
        assert!(s.apply_to_group(1, g1, "Missing").is_err());
        // All Samples reaches everyone, and replaces the group as owner (one owner per gate)
        assert_eq!(s.apply_to_group(0, g, ALL_SAMPLES).unwrap(), 3);
        assert_eq!(s.gate_owner(3, s.find_path(3, &["Lymph".into()]).unwrap()).unwrap().group, ALL_SAMPLES);
        assert_eq!(s.applied.iter().filter(|a| a.path == vec!["Lymph".to_string()]).count(), 1);
    }

    #[test]
    fn editing_in_one_sample_marks_it_edited_and_the_group_can_update_revert_or_release() {
        let mut s = sess(3);
        let g = draw(&mut s, 0, "A", rect(100.0, 600.0));
        s.apply_to_all(0, g).unwrap();
        let n1 = s.find_path(1, &["A".into()]).unwrap();
        assert!(!s.gate_owner(1, n1).unwrap().edited);
        s.edit_gate(1, n1, rect(200.0, 700.0)).unwrap();
        assert!(s.gate_owner(1, n1).unwrap().edited, "overridden in sample 1");
        assert!(!s.gate_owner(0, g).unwrap().edited, "the others are untouched");
        let n2 = s.find_path(2, &["A".into()]).unwrap();
        assert_eq!(s.gate_of(2, n2).unwrap(), rect(100.0, 600.0), "an unsynchronized group does not follow");
        // revert
        s.revert_to_group(1, n1).unwrap();
        assert!(!s.gate_owner(1, n1).unwrap().edited);
        assert_eq!(s.gate_of(1, n1).unwrap(), rect(100.0, 600.0));
        // edit again, then make it the group's version
        s.edit_gate(1, n1, rect(300.0, 800.0)).unwrap();
        s.update_group_from(1, n1).unwrap();
        for (i, n) in [(0, g), (1, n1), (2, n2)] {
            assert_eq!(s.gate_of(i, n).unwrap(), rect(300.0, 800.0));
            assert!(!s.gate_owner(i, n).unwrap().edited);
        }
        // release: the gates stay, ownership goes
        s.release_gate(0, g).unwrap();
        assert!(s.gate_owner(0, g).is_none() && s.gate_owner(2, n2).is_none());
        assert!(s.release_gate(0, g).is_err());
        // an edit that changes the type (rectangle to polygon) is also "edited"
        let h = draw(&mut s, 0, "B", rect(10.0, 50.0));
        s.apply_to_all(0, h).unwrap();
        let hb = s.find_path(1, &["B".into()]).unwrap();
        s.convert_to_polygon(1, hb).unwrap();
        assert!(s.gate_owner(1, hb).unwrap().edited);
        s.revert_to_group(1, hb).unwrap();
        assert!(matches!(s.gate_of(1, hb).unwrap().kind, GateKind::Rectangle), "revert restores the shape family too");
    }

    #[test]
    fn a_synchronized_group_moves_every_copy_together() {
        let mut s = sess(3);
        s.create_group("Sync", &[0, 1]).unwrap();
        s.modify_group("Sync", "Sync", &[0, 1], [9, 9, 9], true).unwrap();
        let g = draw(&mut s, 0, "A", rect(100.0, 600.0));
        s.apply_to_group(0, g, "Sync").unwrap();
        let n1 = s.find_path(1, &["A".into()]).unwrap();
        s.edit_gate(0, g, rect(150.0, 650.0)).unwrap();
        assert_eq!(s.gate_of(1, n1).unwrap(), rect(150.0, 650.0), "the other member followed");
        assert!(!s.gate_owner(1, n1).unwrap().edited && !s.gate_owner(0, g).unwrap().edited);
        assert!(s.find_path(2, &["A".into()]).is_none());
        // All Samples is not synchronized unless asked
        let h = draw(&mut s, 2, "H", rect(5.0, 9.0));
        s.apply_to_all(2, h).unwrap();
        let h0 = s.find_path(0, &["H".into()]).unwrap();
        s.edit_gate(2, h, rect(6.0, 9.0)).unwrap();
        assert_eq!(s.gate_of(0, h0).unwrap(), rect(5.0, 9.0));
        s.all_synced = true;
        s.edit_gate(2, h, rect(7.0, 9.0)).unwrap();
        assert_eq!(s.gate_of(0, h0).unwrap(), rect(7.0, 9.0));
    }

    #[test]
    fn a_group_owned_gate_is_deleted_from_the_group_in_one_of_three_ways() {
        let build = || {
            let mut s = sess(3);
            let g = draw(&mut s, 0, "A", rect(100.0, 600.0));
            s.apply_to_all(0, g).unwrap();
            let n1 = s.find_path(1, &["A".into()]).unwrap();
            s.edit_gate(1, n1, rect(1.0, 2.0)).unwrap(); // overridden in sample 1
            (s, g)
        };
        let (mut s, g) = build();
        assert!(s.delete(0, g).unwrap_err().contains("owned by the group"), "a plain delete is refused");
        assert_eq!(s.delete_from_group(0, g, GroupDelete::Entirely).unwrap(), 2);
        assert!(s.find_path(0, &["A".into()]).is_none() && s.find_path(2, &["A".into()]).is_none());
        assert!(s.find_path(1, &["A".into()]).is_some(), "the overridden copy stays");
        assert!(s.gate_owner(1, s.find_path(1, &["A".into()]).unwrap()).is_none(), "and is no longer owned");
        let (mut s, g) = build();
        assert_eq!(s.delete_from_group(0, g, GroupDelete::KeepInFirstSample).unwrap(), 2);
        assert!(s.find_path(0, &["A".into()]).is_some(), "the first sample keeps its copy");
        assert!(s.find_path(1, &["A".into()]).is_none() && s.find_path(2, &["A".into()]).is_none());
        let (mut s, g) = build();
        assert_eq!(s.delete_from_group(0, g, GroupDelete::ToSamples).unwrap(), 0);
        assert!((0..3).all(|i| s.find_path(i, &["A".into()]).is_some()), "every sample keeps its own copy");
        assert!(s.gate_owner(0, g).is_none());
        // deleting a whole group deals with what it owns the same way
        let (mut s, _) = build();
        s.create_group("G", &[0, 2]).unwrap();
        let n = s.find_path(0, &["A".into()]).unwrap();
        s.apply_to_group(0, n, "G").unwrap();
        assert!(s.delete_group("G", GroupDelete::ToSamples).is_ok());
        assert!(s.group("G").is_none() && s.find_path(2, &["A".into()]).is_some());
    }

    #[test]
    fn renaming_a_group_owned_gate_renames_it_for_all_and_children_keep_their_group() {
        let mut s = sess(3);
        let g = draw(&mut s, 0, "A", rect(100.0, 600.0));
        s.apply_to_all(0, g).unwrap();
        let kid = s.states[0].tree.add_gate(g, "Kid", rect(100.0, 200.0)).unwrap();
        s.apply_to_all(0, kid).unwrap();
        s.rename(0, g, "Renamed").unwrap();
        for i in 0..3 {
            assert!(s.find_path(i, &["Renamed".into()]).is_some() && s.find_path(i, &["A".into()]).is_none(), "sample {i}");
            assert!(s.find_path(i, &["Renamed".into(), "Kid".into()]).is_some());
        }
        assert!(s.gate_owner(1, s.find_path(1, &["Renamed".into(), "Kid".into()]).unwrap()).is_some(), "the child is still owned");
        // a clash in any sample changes nothing
        draw(&mut s, 2, "Taken", rect(1.0, 2.0));
        assert!(s.rename(0, g, "Taken").unwrap_err().contains("already exists"));
        assert!(s.find_path(0, &["Renamed".into()]).is_some());
    }

    #[test]
    fn new_gates_follow_the_draw_target() {
        let mut s = sess(3);
        s.create_group("Pair", &[0, 1]).unwrap();
        let id = s.open_population(0, GateTree::ROOT).unwrap();
        let shape = crate::drawing::Shape::Rect { x0: 100.0, x1: 500.0, y0: 0.0, y1: 250.0 };
        assert_eq!(s.draw_target, DrawTarget::ThisSample, "a bare session keeps gates in the sample they are drawn in");
        s.draw_target = DrawTarget::app_default();
        s.commit_shape(id, &shape, "Everywhere").unwrap();
        assert!((0..3).all(|i| s.find_path(i, &["Everywhere".into()]).is_some()));
        s.draw_target = DrawTarget::Group("Pair".into());
        s.commit_shape(id, &shape, "PairOnly").unwrap();
        assert!(s.find_path(1, &["PairOnly".into()]).is_some() && s.find_path(2, &["PairOnly".into()]).is_none());
        s.draw_target = DrawTarget::ThisSample;
        s.commit_shape(id, &shape, "Mine").unwrap();
        assert!(s.find_path(0, &["Mine".into()]).is_some() && s.find_path(1, &["Mine".into()]).is_none());
        assert!(s.gate_owner(0, s.find_path(0, &["Mine".into()]).unwrap()).is_none());
        // drawing in a sample outside the chosen group keeps the gate in that sample only
        s.draw_target = DrawTarget::Group("Pair".into());
        let id2 = s.open_population(2, GateTree::ROOT).unwrap();
        s.commit_shape(id2, &shape, "Outsider").unwrap();
        assert!(s.find_path(2, &["Outsider".into()]).is_some() && s.find_path(0, &["Outsider".into()]).is_none());
        assert!(s.log.iter().any(|l| l.contains("this sample only")));
    }

    #[test]
    fn groups_and_shared_gates_survive_a_saved_workspace_including_edited_state() {
        let mut s = sess(3);
        s.create_group("Pair", &[0, 2]).unwrap();
        s.modify_group("Pair", "Pair", &[0, 2], [10, 20, 30], true).unwrap();
        let g = draw(&mut s, 0, "A", rect(100.0, 600.0));
        s.apply_to_group(0, g, "Pair").unwrap();
        let h = draw(&mut s, 1, "B", rect(5.0, 9.0));
        s.apply_to_all(1, h).unwrap();
        s.all_synced = false;
        let n2 = s.find_path(2, &["B".into()]).unwrap();
        s.edit_gate(2, n2, rect(6.0, 10.0)).unwrap(); // overridden
        s.draw_target = DrawTarget::Group("Pair".into());
        let json = s.to_workspace_json();
        let mut t = sess(3);
        let notes = t.apply_workspace_json(&json).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(t.groups, s.groups);
        assert_eq!(t.applied, s.applied);
        assert_eq!(t.draw_target, DrawTarget::Group("Pair".into()));
        let n2t = t.find_path(2, &["B".into()]).unwrap();
        assert!(t.gate_owner(2, n2t).unwrap().edited, "still shown as edited after reloading");
        assert_eq!(t.gate_owner(0, t.find_path(0, &["A".into()]).unwrap()).unwrap().color, Some([10, 20, 30]));
        // a workspace from before groups existed loads with none
        let mut old = to_old(&json);
        let mut u = sess(3);
        assert!(u.apply_workspace_json(&mut old).is_ok());
        assert!(u.groups.is_empty() && u.applied.is_empty());
        assert_eq!(u.draw_target, DrawTarget::default());
    }

    fn to_old(j: &Json) -> Json {
        let mut o = Json::obj();
        if let Json::Obj(m) = j {
            for (k, v) in m {
                if k != "groups" {
                    o = o.with(k, v.clone());
                }
            }
        }
        o
    }
}

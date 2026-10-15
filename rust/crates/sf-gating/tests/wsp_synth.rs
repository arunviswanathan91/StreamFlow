//! FlowJo workspace import on a tiny hand-written workspace whose answers can be counted by hand.

use sf_gating::wsp::parse_wsp;
use sf_gating::{EventView, GateTree};

const NS: &str = r#"xmlns:gating="http://www.isac-net.org/std/Gating-ML/v2.0/gating" xmlns:data-type="http://www.isac-net.org/std/Gating-ML/v2.0/datatypes" xmlns:transforms="http://www.isac-net.org/std/Gating-ML/v2.0/transformations""#;

fn workspace() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Workspace {NS}>
 <SampleList>
  <Sample>
   <DataSet uri="file:/C:/data/My%20Run/s1.fcs" sampleID="1"/>
   <Transformations>
    <transforms:linear transforms:minRange="0" transforms:maxRange="10"><data-type:parameter data-type:name="X"/></transforms:linear>
    <transforms:biex transforms:length="256" transforms:maxRange="262144" transforms:neg="0" transforms:width="-10" transforms:pos="4.41854"><data-type:parameter data-type:name="Y"/></transforms:biex>
    <transforms:fasinh transforms:T="1" transforms:A="0" transforms:M="1"><data-type:parameter data-type:name="Z"/></transforms:fasinh>
   </Transformations>
   <SampleNode name="s1.fcs" count="10">
    <Subpopulations>
     <Population name="Box" count="4">
      <Gate id="a"><gating:RectangleGate gating:eventsInside="1">
        <gating:dimension gating:min="2" gating:max="6"><data-type:fcs-dimension data-type:name="X"/></gating:dimension>
        <gating:dimension gating:min="-5" gating:max="50"><data-type:fcs-dimension data-type:name="Y"/></gating:dimension>
      </gating:RectangleGate></Gate>
      <Subpopulations>
       <Population name="HighX" count="2">
        <Gate id="b"><gating:RectangleGate gating:eventsInside="1">
          <gating:dimension gating:min="4"><data-type:fcs-dimension data-type:name="X"/></gating:dimension>
        </gating:RectangleGate></Gate>
       </Population>
       <Population name="Outside" count="2">
        <Gate id="c"><gating:RectangleGate gating:eventsInside="0">
          <gating:dimension gating:min="4"><data-type:fcs-dimension data-type:name="X"/></gating:dimension>
        </gating:RectangleGate></Gate>
       </Population>
      </Subpopulations>
     </Population>
     <Population name="Tri" count="3">
      <Gate id="d"><gating:PolygonGate gating:eventsInside="1">
        <gating:dimension><data-type:fcs-dimension data-type:name="X"/></gating:dimension>
        <gating:dimension><data-type:fcs-dimension data-type:name="W"/></gating:dimension>
        <gating:vertex><gating:coordinate data-type:value="0"/><gating:coordinate data-type:value="0"/></gating:vertex>
        <gating:vertex><gating:coordinate data-type:value="10"/><gating:coordinate data-type:value="0"/></gating:vertex>
        <gating:vertex><gating:coordinate data-type:value="0"/><gating:coordinate data-type:value="10"/></gating:vertex>
      </gating:PolygonGate></Gate>
     </Population>
     <Population name="Round" count="1">
      <Gate id="e"><gating:EllipsoidGate/></Gate>
     </Population>
     <AndNode name="Both"/>
     <Population name="OnZ" count="1">
      <Gate id="f"><gating:RectangleGate><gating:dimension gating:min="0" gating:max="1"><data-type:fcs-dimension data-type:name="Z"/></gating:dimension></gating:RectangleGate></Gate>
     </Population>
    </Subpopulations>
   </SampleNode>
  </Sample>
 </SampleList>
</Workspace>"#
    )
}

/// 10 events, columns X, Y, W: x = 0..9, y = 5 everywhere (below the biex zero? no: 5 raw), w = 9 - x.
fn events() -> (Vec<f32>, Vec<String>) {
    let ch = ["X", "Y", "W"].iter().map(|s| s.to_string()).collect();
    ((0..10).flat_map(|i| [i as f32, 5.0, (9 - i) as f32]).collect(), ch)
}

#[test]
fn counts_match_the_hand_computed_populations_through_the_linear_and_biex_axes() {
    let samples = parse_wsp(&workspace()).unwrap();
    let s = &samples[0];
    assert_eq!(s.file_name.as_deref(), Some("s1.fcs"));
    assert_eq!(s.flowjo_counts[&GateTree::ROOT], 10);
    let (data, ch) = events();
    let (view, names) = s.build_view(&data, &ch).unwrap();
    let ev = s.tree.evaluate(&EventView::new(&view, names.len(), &names).unwrap()).unwrap();
    assert!(ev.warnings.is_empty(), "{:?}", ev.warnings);
    let n = |p: &[&str]| ev.counts[s.tree.find(p).unwrap_or_else(|| panic!("{p:?} missing"))];
    // Box: 2 <= x < 6 (half-open: FlowJo/GatingML) -> x = 2,3,4,5; y = 5 raw is inside the biex-mapped [-5, 50]
    assert_eq!(n(&["Box"]), 4);
    // open-ended upper bound: x >= 4 within Box -> 4,5 ; eventsInside=0 inverts -> 2,3
    assert_eq!(n(&["Box", "HighX"]), 2);
    assert_eq!(n(&["Box", "Outside"]), 2);
    assert_eq!(n(&["Box", "HighX"]) + n(&["Box", "Outside"]), n(&["Box"]), "a gate and its inverse partition the parent");
    // triangle x + w < 10 with w = 9 - x: x + 9 - x = 9 < 10 for every event, on the hypotenuse never; x, w >= 0 -> all 10
    assert_eq!(n(&["Tri"]), 10);
    // FlowJo's saved counts are retained for comparison
    assert_eq!(s.flowjo_counts[&s.tree.find(&["Box"]).unwrap()], 4);
}

#[test]
fn unsupported_constructs_are_skipped_by_name_not_guessed() {
    let s = &parse_wsp(&workspace()).unwrap()[0];
    let all = s.warnings.join("\n");
    for needle in ["Round", "EllipsoidGate", "Both", "AndNode", "fasinh", "OnZ"] {
        assert!(all.contains(needle), "no warning mentioning {needle:?}:\n{all}");
    }
    assert!(s.tree.find(&["Round"]).is_none() && s.tree.find(&["OnZ"]).is_none());
    assert!(s.tree.find(&["Box", "HighX"]).is_some(), "supported siblings survive");
}

#[test]
fn a_missing_channel_is_a_named_error() {
    let s = &parse_wsp(&workspace()).unwrap()[0];
    let data: Vec<f32> = vec![1.0; 10];
    let err = s.build_view(&data, &["X".to_string()]).unwrap_err();
    assert!(err.contains("not in the data file"), "{err}");
}

#[test]
fn bad_documents_are_errors() {
    assert!(parse_wsp("<Workspace>").is_err());
    assert!(parse_wsp("<html/>").unwrap_err().to_string().contains("not a FlowJo workspace"));
    assert!(parse_wsp("<Workspace/>").unwrap_err().to_string().contains("SampleList"));
}

#[test]
fn compensation_prefix_axes_use_the_matrix() {
    let xml = format!(
        r#"<Workspace {NS}><SampleList><Sample>
  <DataSet uri="a.fcs"/>
  <spilloverMatrix prefix="Comp-"><parameters><parameter name="A"/><parameter name="B"/></parameters>
    <spillover parameter="A"><coefficient parameter="A" value="1"/><coefficient parameter="B" value="0.5"/></spillover>
    <spillover parameter="B"><coefficient parameter="A" value="0"/><coefficient parameter="B" value="1"/></spillover>
  </spilloverMatrix>
  <SampleNode name="a.fcs" count="1"><Subpopulations><Population name="P" count="1"><Gate id="g">
   <gating:RectangleGate><gating:dimension gating:min="-0.1" gating:max="0.1"><data-type:fcs-dimension data-type:name="Comp-B"/></gating:dimension></gating:RectangleGate>
  </Gate></Population></Subpopulations></SampleNode></Sample></SampleList></Workspace>"#
    );
    let s = &parse_wsp(&xml).unwrap()[0];
    // one event: A = 100, B = 50 (= 0.5 * A spill). compensated = raw . inv(S): B' = 50 - 0.5 * 100 = 0
    let (data, ch) = (vec![100.0f32, 50.0], vec!["A".to_string(), "B".to_string()]);
    let (view, names) = s.build_view(&data, &ch).unwrap();
    assert_eq!(names, vec!["Comp-B"]);
    assert!(view[0].abs() < 1e-4, "compensated B should be ~0, got {}", view[0]);
    let ev = s.tree.evaluate(&EventView::new(&view, 1, &names).unwrap()).unwrap();
    assert_eq!(ev.counts[s.tree.find(&["P"]).unwrap()], 1);
}

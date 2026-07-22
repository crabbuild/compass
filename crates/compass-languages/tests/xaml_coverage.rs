use std::error::Error;
use std::fs;

use compass_languages::Engine;

#[test]
fn xaml_project_covers_codebehind_events_viewmodels_toolkit_members_and_bindings()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("Fixture.csproj"), "<Project />")?;
    fs::create_dir_all(directory.path().join("obj"))?;
    fs::write(
        directory.path().join("obj/IgnoredViewModel.cs"),
        "public class IgnoredViewModel {}",
    )?;
    fs::write(
        directory.path().join("MainWindow.xaml.cs"),
        r#"
namespace App;
public partial class MainWindow {
    public void OnClick(object? sender, RoutedEventArgs e) { }
    public void NotAnEvent() { }
}
public partial class MainWindowViewModel {
    [ObservableProperty]
    private string _title;

    [ObservableProperty] private int m_count;

    [RelayCommand]
    private async Task SaveAsync() { }

    [RelayCommand] private void Cancel() { }
}
"#,
    )?;
    let xaml = directory.path().join("MainWindow.xaml");
    fs::write(
        &xaml,
        r#"<Window xmlns="urn:fixture" xmlns:x="urn:x" xmlns:local="urn:local" x:Class="App.MainWindow">
  <Window.DataContext><local:MainWindowViewModel /></Window.DataContext>
  <Button x:Name="SaveButton" Click="OnClick" Command="{Binding SaveCommand}" Content="Save" />
  <TextBlock x:Name="TitleText" Text="{Binding Path=Title, Converter={StaticResource ResourceKey=TitleConverter}}" />
  <Binding Path="Count" Converter="{StaticResource CountConverter}" />
</Window>"#,
    )?;

    let mut engine = Engine::default();
    let extraction = engine.extract(&xaml)?;
    assert!(extraction.error.is_none());
    for label in [
        "MainWindow",
        "MainWindowViewModel",
        "SaveButton",
        "TitleText",
        ".OnClick()",
        "Title",
        "Count",
        "SaveCommand",
        "CancelCommand",
        "TitleConverter",
        "CountConverter",
    ] {
        assert!(
            extraction.nodes.iter().any(|node| node.label() == label),
            "missing {label}; labels={:?}",
            extraction
                .nodes
                .iter()
                .map(|node| node.label())
                .collect::<Vec<_>>()
        );
    }
    assert!(extraction.edges.iter().any(|edge| {
        edge.attributes
            .get("context")
            .and_then(serde_json::Value::as_str)
            == Some("communitytoolkit_observable_property")
    }));
    assert!(extraction.edges.iter().any(|edge| {
        edge.attributes
            .get("context")
            .and_then(serde_json::Value::as_str)
            == Some("binding_converter")
    }));
    Ok(())
}

#[test]
fn xaml_inference_design_instance_prism_and_case_insensitive_codebehind_are_supported()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(
        directory.path().join("Fixture.sln"),
        "Microsoft Visual Studio Solution File",
    )?;
    fs::write(
        directory.path().join("SettingsView.XAML.CS"),
        r#"namespace App { public partial class SettingsView { public void LoadedHandler(object sender, EventArgs e) {} } public class SettingsViewModel { public string Name { get; set; } } }"#,
    )?;
    fs::write(
        directory.path().join("Other.cs"),
        "namespace App { public class MainViewModel {} public class MainWindowViewModel {} }",
    )?;
    let xaml = directory.path().join("SettingsView.xaml");
    fs::write(
        &xaml,
        r#"<UserControl xmlns="urn:fixture" xmlns:x="urn:x" xmlns:d="urn:d" xmlns:prism="urn:prism" x:Class="App.SettingsView" d:DataContext="{d:DesignInstance Type={x:Type App.SettingsViewModel}}" prism:ViewModelLocator.AutoWireViewModel="TRUE" Loaded="LoadedHandler"><TextBlock Text="{Binding Name}" /></UserControl>"#,
    )?;
    let extraction = Engine::default().extract(&xaml)?;
    assert!(extraction.error.is_none());
    assert!(
        extraction
            .nodes
            .iter()
            .any(|node| node.label() == "SettingsViewModel")
    );
    assert!(
        extraction
            .nodes
            .iter()
            .any(|node| node.label() == ".LoadedHandler()"),
        "labels={:?}",
        extraction
            .nodes
            .iter()
            .map(|node| node.label())
            .collect::<Vec<_>>()
    );

    let prism = directory.path().join("DashboardPage.xaml");
    fs::write(
        &prism,
        r#"<Page xmlns="urn:fixture" xmlns:prism="urn:prism" prism:ViewModelLocator.AutoWireViewModel="true"><Label Text="{Binding Name}" /></Page>"#,
    )?;
    let extraction = Engine::default().extract(&prism)?;
    assert!(extraction.error.is_none());
    Ok(())
}

#[test]
fn extraction_limits_xml_security_parse_failures_and_missing_sources_are_structured()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let mut engine = Engine::default();

    let oversized = directory.path().join("oversized.xaml");
    fs::write(&oversized, vec![b'x'; 2 * 1024 * 1024 + 1])?;
    assert_eq!(
        engine.extract(&oversized)?.error.as_deref(),
        Some("xaml file too large")
    );

    let doctype = directory.path().join("doctype.xaml");
    fs::write(&doctype, "<!DOCTYPE x [<!ENTITY y 'z'>]><Page />")?;
    assert!(
        engine
            .extract(&doctype)?
            .error
            .as_deref()
            .is_some_and(|error| error.contains("DOCTYPE/ENTITY"))
    );

    let malformed = directory.path().join("malformed.xaml");
    fs::write(&malformed, "<Page><Broken></Page>")?;
    assert!(
        engine
            .extract(&malformed)?
            .error
            .as_deref()
            .is_some_and(|error| error.contains("XML parse error"))
    );

    let huge_json = directory.path().join("huge.json");
    fs::write(&huge_json, vec![b' '; 1024 * 1024 + 1])?;
    assert_eq!(
        engine.extract(&huge_json)?.error.as_deref(),
        Some("json file too large to index")
    );

    for missing in ["missing.xaml", "missing.json", "missing.tf", "missing.dm"] {
        assert!(
            engine.extract(&directory.path().join(missing)).is_err(),
            "{missing}"
        );
    }
    Ok(())
}

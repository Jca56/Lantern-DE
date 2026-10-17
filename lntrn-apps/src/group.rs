//! The groups a launcher sorts apps under: the freedesktop main
//! categories, a few run together (Audio and Video are Multimedia), named
//! as people say them. An app goes under every main category its file
//! lists, as the freedesktop menu has it (Steam is a game and an internet
//! app), and under Other when it lists none.

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    Development,
    Education,
    Games,
    Graphics,
    Internet,
    Multimedia,
    Office,
    Science,
    Settings,
    System,
    Utilities,
    Other,
}

impl Group {
    /// In the order a launcher lists them: by name, Other last.
    pub const ALL: [Group; 12] = [Group::Development, Group::Education, Group::Games, Group::Graphics, Group::Internet, Group::Multimedia, Group::Office, Group::Science, Group::Settings, Group::System, Group::Utilities, Group::Other];

    pub fn label(self) -> &'static str {
        match self {
            Group::Development => "Development",
            Group::Education => "Education",
            Group::Games => "Games",
            Group::Graphics => "Graphics",
            Group::Internet => "Internet",
            Group::Multimedia => "Multimedia",
            Group::Office => "Office",
            Group::Science => "Science",
            Group::Settings => "Settings",
            Group::System => "System",
            Group::Utilities => "Utilities",
            Group::Other => "Other",
        }
    }

    /// The groups an app with these freedesktop `categories` goes under,
    /// in [`Self::ALL`]'s order.
    pub fn of(categories: &[String]) -> Vec<Group> {
        let mut groups: Vec<Group> = categories.iter().filter_map(|c| main(c)).collect();
        groups.sort();
        groups.dedup();
        if groups.is_empty() {
            groups.push(Group::Other);
        }
        groups
    }
}

fn main(category: &str) -> Option<Group> {
    Some(match category {
        "Development" => Group::Development,
        "Education" => Group::Education,
        "Game" => Group::Games,
        "Graphics" => Group::Graphics,
        "Network" => Group::Internet,
        "AudioVideo" | "Audio" | "Video" => Group::Multimedia,
        "Office" => Group::Office,
        "Science" => Group::Science,
        "Settings" => Group::Settings,
        "System" => Group::System,
        "Utility" => Group::Utilities,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(cats: &[&str]) -> Vec<Group> {
        Group::of(&cats.iter().map(|c| (*c).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn an_app_goes_under_every_main_category_it_lists() {
        assert_eq!(of(&["Network", "FileTransfer", "Game"]), vec![Group::Games, Group::Internet]);
        assert_eq!(of(&["GTK", "AudioVideo", "Audio", "Player"]), vec![Group::Multimedia], "once, past what is no main category");
        assert_eq!(of(&["System", "Settings"]), vec![Group::Settings, Group::System]);
        assert_eq!(of(&["ConsoleOnly"]), vec![Group::Other]);
        assert_eq!(of(&[]), vec![Group::Other]);
    }

    #[test]
    fn the_groups_are_listed_by_name_with_other_last() {
        let labels: Vec<&str> = Group::ALL.iter().map(|g| g.label()).collect();
        let mut sorted = labels[..labels.len() - 1].to_vec();
        sorted.sort();
        assert_eq!(&labels[..labels.len() - 1], &sorted[..]);
        assert_eq!(labels.last(), Some(&"Other"));
    }
}

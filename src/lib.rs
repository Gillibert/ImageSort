//! ImageSort : classe des images en les déplaçant dans des sous-répertoires.
//!
//! La logique est découpée en modules indépendants de l'interface graphique
//! pour pouvoir être testée unitairement :
//! - [`formats`] : extensions d'images reconnues ;
//! - [`natural`] : tri « naturel » (img2 avant img10) ;
//! - [`scan`]    : découverte des images et des répertoires cibles ;
//! - [`hotkeys`] : attribution des touches aux répertoires ;
//! - [`session`] : état du tri (déplacement, suppression, annulation) ;
//! - [`prefetch`]: préchargement multi-thread ;
//! - [`decode`]  : décodage + réduction à la taille de l'écran ;
//! - [`view`]    : géométrie d'affichage.

pub mod app;
pub mod decode;
pub mod formats;
pub mod hotkeys;
pub mod natural;
pub mod prefetch;
pub mod scan;
pub mod session;
pub mod view;

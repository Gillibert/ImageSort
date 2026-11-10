//! Préchargement multi-thread.
//!
//! L'appelant indique la liste ordonnée des chemins voulus (le plus prioritaire
//! en premier) ; des threads de travail les chargent dans cet ordre. Tout ce qui
//! n'est plus voulu est évincé du cache, ce qui borne la mémoire.

use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;

type Loader<T> = dyn Fn(&Path) -> Result<T, String> + Send + Sync;
type OnReady = dyn Fn() + Send + Sync;

struct State<T> {
    wanted: Vec<PathBuf>,
    ready: HashMap<PathBuf, Arc<Result<T, String>>>,
    in_flight: HashSet<PathBuf>,
    /// Incrémenté par `clear` : les résultats des chargements en cours d'une génération périmée sont jetés.
    generation: u64,
    shutdown: bool,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    wake: Condvar,
    loader: Box<Loader<T>>,
    on_ready: Box<OnReady>,
}

impl<T> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

pub struct Prefetcher<T: Send + Sync + 'static> {
    shared: Arc<Shared<T>>,
}

impl<T: Send + Sync + 'static> Prefetcher<T> {
    /// `loader` est appelé depuis les threads de travail ; `on_ready` après chaque
    /// chargement terminé (typiquement : demander un rafraîchissement de l'écran).
    pub fn new(
        workers: usize,
        loader: impl Fn(&Path) -> Result<T, String> + Send + Sync + 'static,
        on_ready: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                wanted: Vec::new(),
                ready: HashMap::new(),
                in_flight: HashSet::new(),
                generation: 0,
                shutdown: false,
            }),
            wake: Condvar::new(),
            loader: Box::new(loader),
            on_ready: Box::new(on_ready),
        });
        for i in 0..workers.max(1) {
            let shared = Arc::clone(&shared);
            thread::Builder::new()
                .name(format!("prefetch-{i}"))
                .spawn(move || worker(shared))
                .expect("impossible de lancer un thread de préchargement");
        }
        Prefetcher { shared }
    }

    /// Remplace la liste des chemins voulus (ordre = priorité) et évince le reste.
    pub fn set_wanted(&self, paths: Vec<PathBuf>) {
        let mut st = self.shared.lock();
        if st.wanted == paths {
            return;
        }
        let keep: HashSet<&PathBuf> = paths.iter().collect();
        st.ready.retain(|p, _| keep.contains(p));
        st.wanted = paths;
        drop(st);
        self.shared.wake.notify_all();
    }

    /// Résultat déjà chargé pour ce chemin, s'il existe.
    pub fn get(&self, path: &Path) -> Option<Arc<Result<T, String>>> {
        self.shared.lock().ready.get(path).cloned()
    }

    /// Oublie tout ce qui est chargé et recharge (ex. changement de taille d'écran).
    pub fn clear(&self) {
        let mut st = self.shared.lock();
        st.ready.clear();
        st.generation += 1;
        drop(st);
        self.shared.wake.notify_all();
    }

    /// Nombre d'éléments actuellement en cache.
    pub fn cached(&self) -> usize {
        self.shared.lock().ready.len()
    }
}

impl<T: Send + Sync + 'static> Drop for Prefetcher<T> {
    fn drop(&mut self) {
        self.shared.lock().shutdown = true;
        self.shared.wake.notify_all();
    }
}

fn worker<T: Send + Sync + 'static>(shared: Arc<Shared<T>>) {
    loop {
        // Choisir le prochain chemin voulu qui n'est ni chargé ni en cours.
        let (path, generation) = {
            let mut st = shared.lock();
            loop {
                if st.shutdown {
                    return;
                }
                let next = st.wanted.iter().find(|p| !st.ready.contains_key(*p) && !st.in_flight.contains(*p)).cloned();
                if let Some(p) = next {
                    st.in_flight.insert(p.clone());
                    break (p, st.generation);
                }
                st = shared.wake.wait(st).unwrap_or_else(|p| p.into_inner());
            }
        };

        let result = catch_unwind(AssertUnwindSafe(|| (shared.loader)(&path)))
            .unwrap_or_else(|_| Err("le décodage a planté".to_string()));

        let mut st = shared.lock();
        st.in_flight.remove(&path);
        let still_wanted = st.generation == generation && st.wanted.contains(&path);
        if still_wanted {
            st.ready.insert(path, Arc::new(result));
            drop(st);
            (shared.on_ready)();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn wait_until(mut cond: impl FnMut() -> bool) {
        let start = Instant::now();
        while !cond() {
            assert!(start.elapsed() < Duration::from_secs(5), "délai dépassé");
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn loads_wanted_paths_and_serves_them() {
        let pf = Prefetcher::new(2, |path| Ok(path.to_string_lossy().to_uppercase()), || {});
        pf.set_wanted(vec![p("a"), p("b"), p("c")]);
        wait_until(|| pf.cached() == 3);
        assert_eq!(*pf.get(Path::new("b")).unwrap(), Ok("B".to_string()));
        assert!(pf.get(Path::new("zzz")).is_none());
    }

    #[test]
    fn errors_are_cached_and_panics_are_contained() {
        let pf = Prefetcher::new(
            1,
            |path| match path.to_str().unwrap() {
                "bad" => Err("illisible".to_string()),
                "boom" => panic!("décodeur cassé"),
                other => Ok(other.to_string()),
            },
            || {},
        );
        pf.set_wanted(vec![p("bad"), p("boom"), p("ok")]);
        wait_until(|| pf.cached() == 3);
        assert_eq!(*pf.get(Path::new("bad")).unwrap(), Err("illisible".to_string()));
        assert!(pf.get(Path::new("boom")).unwrap().is_err());
        assert_eq!(*pf.get(Path::new("ok")).unwrap(), Ok("ok".to_string()));
    }

    #[test]
    fn processes_in_priority_order() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let o = order.clone();
        let pf = Prefetcher::new(
            1, // un seul thread => ordre observable
            move |path| {
                o.lock().unwrap().push(path.to_string_lossy().into_owned());
                Ok(())
            },
            || {},
        );
        pf.set_wanted(vec![p("1"), p("2"), p("3"), p("4")]);
        wait_until(|| pf.cached() == 4);
        assert_eq!(*order.lock().unwrap(), vec!["1", "2", "3", "4"]);
    }

    #[test]
    fn unwanted_entries_are_evicted_and_not_reloaded_needlessly() {
        let loads = Arc::new(AtomicUsize::new(0));
        let l = loads.clone();
        let pf = Prefetcher::new(
            2,
            move |path| {
                l.fetch_add(1, Ordering::SeqCst);
                Ok(path.to_string_lossy().into_owned())
            },
            || {},
        );
        pf.set_wanted(vec![p("a"), p("b"), p("c")]);
        wait_until(|| pf.cached() == 3);
        assert_eq!(loads.load(Ordering::SeqCst), 3);

        // Fenêtre glissante : a disparaît, d apparaît ; b et c ne sont pas rechargés.
        pf.set_wanted(vec![p("b"), p("c"), p("d")]);
        wait_until(|| pf.get(Path::new("d")).is_some());
        assert!(pf.get(Path::new("a")).is_none());
        assert_eq!(pf.cached(), 3);
        assert_eq!(loads.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn a_result_arriving_after_eviction_is_discarded() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let rx = Mutex::new(rx);
        let started = Arc::new(AtomicUsize::new(0));
        let s = started.clone();
        let pf = Prefetcher::new(
            1,
            move |path| {
                if path == Path::new("slow") {
                    s.fetch_add(1, Ordering::SeqCst);
                    rx.lock().unwrap().recv().ok(); // bloque jusqu'à autorisation
                }
                Ok(path.to_string_lossy().into_owned())
            },
            || {},
        );
        pf.set_wanted(vec![p("slow")]);
        wait_until(|| started.load(Ordering::SeqCst) == 1);
        pf.set_wanted(vec![p("other")]); // slow n'est plus voulu pendant son chargement
        tx.send(()).unwrap();
        wait_until(|| pf.get(Path::new("other")).is_some());
        assert!(pf.get(Path::new("slow")).is_none());
        assert_eq!(pf.cached(), 1);
    }

    #[test]
    fn clear_reloads_everything() {
        let loads = Arc::new(AtomicUsize::new(0));
        let l = loads.clone();
        let pf = Prefetcher::new(
            2,
            move |_| {
                l.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            || {},
        );
        pf.set_wanted(vec![p("a"), p("b")]);
        wait_until(|| pf.cached() == 2);
        pf.clear();
        wait_until(|| loads.load(Ordering::SeqCst) == 4 && pf.cached() == 2);
    }

    #[test]
    fn on_ready_is_called_for_each_load() {
        let ready = Arc::new(AtomicUsize::new(0));
        let r = ready.clone();
        let pf = Prefetcher::new(2, |_| Ok(()), move || {
            r.fetch_add(1, Ordering::SeqCst);
        });
        pf.set_wanted(vec![p("a"), p("b"), p("c")]);
        wait_until(|| ready.load(Ordering::SeqCst) == 3);
    }

    #[test]
    fn work_runs_in_parallel() {
        // 4 chargements de 100 ms sur 4 threads doivent prendre bien moins de 400 ms.
        let pf = Prefetcher::new(
            4,
            |_| {
                thread::sleep(Duration::from_millis(100));
                Ok(())
            },
            || {},
        );
        let start = Instant::now();
        pf.set_wanted(vec![p("1"), p("2"), p("3"), p("4")]);
        wait_until(|| pf.cached() == 4);
        assert!(start.elapsed() < Duration::from_millis(350), "pas de parallélisme : {:?}", start.elapsed());
    }
}

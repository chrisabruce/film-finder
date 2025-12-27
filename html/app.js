// Film Finder - Interactive functionality

document.addEventListener('DOMContentLoaded', () => {
    const movieCards = document.querySelectorAll('.movie-card');
    const theaterCheckboxes = document.querySelectorAll('input[name="theater"]');
    const selectAllBtn = document.getElementById('select-all');
    const selectNoneBtn = document.getElementById('select-none');
    const showAllOvCheckbox = document.getElementById('show-all-ov');
    const searchInput = document.getElementById('search');

    // Movie card expansion
    movieCards.forEach(card => {
        card.addEventListener('click', (e) => {
            // Don't collapse if clicking a link or input
            if (e.target.tagName === 'A' || e.target.tagName === 'INPUT') return;

            const isExpanded = card.classList.contains('expanded');

            // Collapse all other cards
            movieCards.forEach(c => {
                c.classList.remove('expanded');
                const screenings = c.querySelector('.movie-screenings');
                if (screenings) screenings.hidden = true;
            });

            // Toggle this card
            if (!isExpanded) {
                card.classList.add('expanded');
                const screenings = card.querySelector('.movie-screenings');
                if (screenings) {
                    screenings.hidden = false;
                    // Scroll the card into view
                    setTimeout(() => {
                        card.scrollIntoView({ behavior: 'smooth', block: 'start' });
                    }, 100);
                }
            }

            updateScreeningsVisibility();
        });
    });

    // Filter functionality (theaters + English/all OV + search)
    function updateMoviesVisibility() {
        const selectedTheaters = new Set(
            Array.from(theaterCheckboxes)
                .filter(cb => cb.checked)
                .map(cb => cb.value)
        );
        const showAllOv = showAllOvCheckbox?.checked || false;
        const searchQuery = (searchInput?.value || '').toLowerCase().trim();

        movieCards.forEach(card => {
            const movieTheaters = card.dataset.theaters.split(',');
            const hasSelectedTheater = movieTheaters.some(t => selectedTheaters.has(t));
            const isEnglish = card.dataset.english === 'true';
            const searchText = card.dataset.search || '';

            // Check search match
            const matchesSearch = searchQuery === '' || searchText.includes(searchQuery);

            // Default is English only; if "Show all OV" is checked, show all
            const matchesLanguage = showAllOv || isEnglish;

            // Hide if no selected theater OR doesn't match language filter OR doesn't match search
            const shouldHide = !hasSelectedTheater || !matchesLanguage || !matchesSearch;
            card.classList.toggle('hidden', shouldHide);
        });

        updateScreeningsVisibility();
    }

    function updateScreeningsVisibility() {
        const selectedTheaters = new Set(
            Array.from(theaterCheckboxes)
                .filter(cb => cb.checked)
                .map(cb => cb.value)
        );

        document.querySelectorAll('.screening').forEach(screening => {
            const theaterId = screening.dataset.theater;
            screening.classList.toggle('hidden', !selectedTheaters.has(theaterId));
        });
    }

    theaterCheckboxes.forEach(cb => {
        cb.addEventListener('change', updateMoviesVisibility);
    });

    showAllOvCheckbox?.addEventListener('change', updateMoviesVisibility);

    searchInput?.addEventListener('input', updateMoviesVisibility);

    selectAllBtn?.addEventListener('click', () => {
        theaterCheckboxes.forEach(cb => cb.checked = true);
        updateMoviesVisibility();
    });

    selectNoneBtn?.addEventListener('click', () => {
        theaterCheckboxes.forEach(cb => cb.checked = false);
        updateMoviesVisibility();
    });

    // Keyboard navigation
    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') {
            // Clear search if focused
            if (document.activeElement === searchInput) {
                searchInput.value = '';
                updateMoviesVisibility();
                return;
            }
            // Otherwise collapse expanded cards
            movieCards.forEach(card => {
                card.classList.remove('expanded');
                const screenings = card.querySelector('.movie-screenings');
                if (screenings) screenings.hidden = true;
            });
        }
    });

    // Initial filter (English only by default)
    updateMoviesVisibility();
});

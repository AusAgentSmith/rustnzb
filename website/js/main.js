// Navigation scroll effect
var navbar = document.getElementById('navbar');
var navToggle = document.getElementById('navToggle');
var navMenu = document.getElementById('navMenu');

window.addEventListener('scroll', function() {
    if (window.scrollY > 20) {
        navbar.classList.add('scrolled');
    } else {
        navbar.classList.remove('scrolled');
    }
});

// Mobile menu toggle
navToggle.addEventListener('click', function() {
    navToggle.classList.toggle('active');
    navMenu.classList.toggle('open');
});

// Close mobile menu on link click
navMenu.querySelectorAll('a').forEach(function(link) {
    link.addEventListener('click', function() {
        navToggle.classList.remove('active');
        navMenu.classList.remove('open');
    });
});

// Tab switching for Getting Started
function switchTab(tab) {
    document.querySelectorAll('.start-tab').forEach(function(btn) {
        btn.classList.toggle('active', btn.getAttribute('data-tab') === tab);
    });
    document.querySelectorAll('.start-pane').forEach(function(pane) {
        pane.classList.toggle('active', pane.id === 'pane-' + tab);
    });
}

// Active nav link on scroll
var sections = document.querySelectorAll('section[id]');

function updateActiveNav() {
    var scrollPos = window.scrollY + 100;
    sections.forEach(function(section) {
        var top = section.offsetTop;
        var height = section.offsetHeight;
        var id = section.getAttribute('id');
        var link = document.querySelector('.nav-menu a[href="#' + id + '"]');
        if (link) {
            if (scrollPos >= top && scrollPos < top + height) {
                link.style.color = 'var(--text-primary)';
            } else {
                link.style.color = '';
            }
        }
    });
}

window.addEventListener('scroll', updateActiveNav);

// Smooth reveal on scroll
var observer = new IntersectionObserver(function(entries) {
    entries.forEach(function(entry) {
        if (entry.isIntersecting) {
            entry.target.style.opacity = '1';
            entry.target.style.transform = 'translateY(0)';
        }
    });
}, { threshold: 0.1 });

document.addEventListener('DOMContentLoaded', function() {
    var cards = document.querySelectorAll('.feature-card, .arch-card, .pipeline-step, .bench-info-card, .bench-stat-card');
    cards.forEach(function(card) {
        card.style.opacity = '0';
        card.style.transform = 'translateY(20px)';
        card.style.transition = 'opacity 0.4s ease, transform 0.4s ease';
        observer.observe(card);
    });
});

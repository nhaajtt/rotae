// Applied before first paint so the page never flashes the wrong theme.
(function () {
  var saved = localStorage.getItem('theme');
  var dark = saved !== 'light';
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
})();

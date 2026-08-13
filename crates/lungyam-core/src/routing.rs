use std::cmp::Ordering;

use crate::config::RouteConfig;

/// Sorts routes in the same order used by the proxy data plane.
pub fn sort_routes(routes: &mut [RouteConfig]) {
    routes.sort_by(route_ordering);
}

/// Returns whether a route matches an incoming host, path, and method.
#[must_use]
pub fn route_matches(
    route: &RouteConfig,
    host: Option<&str>,
    path: &str,
    method: &str,
) -> bool {
    host_matches(route.host.as_deref(), host)
        && path_matches(&route.path, path)
        && (route.methods.is_empty()
            || route
                .methods
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(method)))
}

/// Finds the first route using the exact data-plane evaluation order.
#[must_use]
pub fn find_matching_route<'a>(
    routes: &'a [RouteConfig],
    host: Option<&str>,
    path: &str,
    method: &str,
) -> Option<&'a RouteConfig> {
    let mut ordered: Vec<&RouteConfig> = routes.iter().collect();
    ordered.sort_by(|left, right| route_ordering(left, right));
    ordered
        .into_iter()
        .find(|route| route_matches(route, host, path, method))
}

fn route_ordering(left: &RouteConfig, right: &RouteConfig) -> Ordering {
    right
        .priority
        .cmp(&left.priority)
        .then_with(|| right.path.len().cmp(&left.path.len()))
}

fn host_matches(expected: Option<&str>, actual: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let Some(actual) = actual else {
        return false;
    };

    actual.eq_ignore_ascii_case(expected)
        || actual
            .strip_prefix(expected)
            .is_some_and(|suffix| suffix.starts_with(':'))
}

fn path_matches(route_path: &str, actual: &str) -> bool {
    if route_path == "/" || route_path == actual {
        return true;
    }

    actual
        .strip_prefix(route_path)
        .is_some_and(|suffix| suffix.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use crate::config::{RouteConfig, RoutePolicies};

    use super::{find_matching_route, route_matches, sort_routes};

    #[test]
    fn matching_respects_host_port_path_boundary_and_method() {
        let route = route("api", Some("api.example.com"), "/api", &["POST"], 0);

        assert!(route_matches(
            &route,
            Some("api.example.com:8080"),
            "/api/users",
            "post"
        ));
        assert!(!route_matches(
            &route,
            Some("api.example.com"),
            "/apiv2",
            "POST"
        ));
        assert!(!route_matches(
            &route,
            Some("api.example.com"),
            "/api",
            "GET"
        ));
    }

    #[test]
    fn first_match_uses_priority_then_path_specificity() {
        let routes = vec![
            route("fallback", None, "/", &[], 100),
            route("specific", None, "/api", &[], 100),
            route("lower", None, "/api/users", &[], 50),
        ];

        let matched = find_matching_route(&routes, None, "/api/users", "GET")
            .expect("a route should match");
        assert_eq!(matched.name, "specific");
    }

    #[test]
    fn sorting_preserves_configuration_order_for_equal_rank() {
        let mut routes = vec![
            route("first", None, "/one", &[], 10),
            route("second", None, "/two", &[], 10),
        ];

        sort_routes(&mut routes);
        assert_eq!(routes[0].name, "first");
        assert_eq!(routes[1].name, "second");
    }

    fn route(
        name: &str,
        host: Option<&str>,
        path: &str,
        methods: &[&str],
        priority: i32,
    ) -> RouteConfig {
        RouteConfig {
            name: name.to_owned(),
            host: host.map(ToOwned::to_owned),
            path: path.to_owned(),
            methods: methods.iter().map(|method| (*method).to_owned()).collect(),
            upstream: "api".to_owned(),
            priority,
            policies: RoutePolicies::default(),
        }
    }
}

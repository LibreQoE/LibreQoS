import html from './template.html';
import { Page } from '../page'
import { MenuPage } from '../menu/menu';
import { Component } from '../components/component';
import { ThroughputSiteChart } from '../components/throughput_site';
import { SiteInfo } from '../components/site_info';
import { RttChartSite } from '../components/rtt_site';
import { RttHistoSite } from '../components/rtt_histo_site';
import { SiteBreadcrumbs } from '../components/site_breadcrumbs';
import { SiteHeat } from '../components/site_heat';
import { SiteStackChart } from '../components/site_stack';

export class SitePage implements Page {
    menu: MenuPage;
    components: Component[];
    siteId: string;

    constructor(siteId: string) {
        this.siteId = siteId;
        this.menu = new MenuPage("sitetreeDash");
        let container = document.getElementById('mainContent');
        if (container) {
            container.innerHTML = html;
        }
        this.components = [
            new SiteInfo(siteId),
            new ThroughputSiteChart(siteId),
            new RttChartSite(siteId),
            new RttHistoSite(),
            new SiteBreadcrumbs(siteId),
            new SiteHeat(siteId),
            new SiteStackChart(siteId),
        ];
    }

    wireup() {
        this.components.forEach(component => {
            component.wireup();
        });
    }

    ontick(): void {
        this.menu.ontick();
        this.components.forEach(component => {
            component.ontick();
        });
    }

    onmessage(event: any) {
        if (event.msg) {
            this.menu.onmessage(event);

            this.components.forEach(component => {
                component.onmessage(event);
            });
        }
    }
}
